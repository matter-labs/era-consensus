use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use zksync_concurrency::{ctx, error::Wrap as _, metrics::LatencyHistogramExt as _, sync, time};
use zksync_consensus_roles::{validator, validator::ConsensusMsg};
use zksync_consensus_roles::validator::Signed;
use zksync_consensus_storage as storage;

use crate::{Config, metrics, OutputSender};

/// The StateMachine struct contains the state of the replica. This is the most complex state machine and is responsible
/// for validating and voting on blocks. When participating in consensus we are always a replica.
#[derive(Debug)]
pub(crate) struct StateMachine {
    /// Consensus configuration and output channel.
    pub(crate) config: Arc<Config>,
    /// Pipe through which replica sends network messages.
    pub(super) outbound_pipe: OutputSender,
    /// Pipe through which replica receives network messages.
    inbound_pipe: sync::prunable_mpsc::Receiver<
        Option<Signed<ConsensusMsg>>,
        ctx::Result<()>
    >,
    /// The current view number.
    pub(crate) view: validator::ViewNumber,
    /// The current phase.
    pub(crate) phase: validator::Phase,
    /// The highest block proposal that the replica has committed to.
    pub(crate) high_vote: validator::ReplicaCommit,
    /// The highest commit quorum certificate known to the replica.
    pub(crate) high_qc: validator::CommitQC,
    /// A cache of the received block proposals.
    pub(crate) block_proposal_cache:
    BTreeMap<validator::BlockNumber, HashMap<validator::PayloadHash, validator::Payload>>,
    /// The deadline to receive an input message.
    pub(crate) timeout_deadline: sync::watch::Sender<time::Deadline>,
}

impl StateMachine {
    /// Creates a new StateMachine struct. We try to recover a past state from the storage module,
    /// otherwise we initialize the state machine with whatever head block we have.
    pub(crate) async fn start(ctx: &ctx::Ctx, config: Arc<Config>, outbound_pipe: OutputSender)
        -> ctx::Result<(
            Self,
            sync::prunable_mpsc::Sender<Option<Signed<ConsensusMsg>>, ctx::Result<()>>
        )> {
        let backup = match config.replica_store.state(ctx).await? {
            Some(backup) => backup,
            None => config.block_store.subscribe().borrow().last.clone().into(),
        };
        let mut block_proposal_cache: BTreeMap<_, HashMap<_, _>> = BTreeMap::new();
        for proposal in backup.proposals {
            block_proposal_cache
                .entry(proposal.number)
                .or_default()
                .insert(proposal.payload.hash(), proposal.payload);
        }

        let (send, recv) =
            sync::prunable_mpsc::channel(StateMachine::inbound_pruning_predicate);

        let mut this = Self {
            config,
            outbound_pipe,
            inbound_pipe: recv,
            view: backup.view,
            phase: backup.phase,
            high_vote: backup.high_vote,
            high_qc: backup.high_qc,
            block_proposal_cache,
            timeout_deadline: sync::watch::channel(time::Deadline::Infinite).0,
        };

        // We need to start the replica before processing inputs.
        this.start_new_view(ctx).await.wrap("start_new_view()")?;

        Ok((this, send))
    }

    /// Runs a loop to process incoming messages (may be `None` if the channel times out while waiting for a message).
    /// This is the main entry point for the state machine,
    /// potentially triggering state modifications and message sending to the executor.
    pub async fn run(mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        loop {
            let (signed_message, res_send) = self.inbound_pipe.recv(ctx).await?;
            let Some(signed_message) = signed_message else {
                let res = self.start_new_view(ctx).await;
                let _ = res_send.send(res);
                continue;
            };

            let now = ctx.now();
            let label = match &signed_message.msg {
                ConsensusMsg::LeaderPrepare(_) => {
                    let res = match self
                        .process_leader_prepare(ctx, signed_message.cast().unwrap())
                        .await
                        .wrap("process_leader_prepare()")
                    {
                        Err(super::leader_prepare::Error::Internal(err)) => return Err(err),
                        Err(err) => {
                            tracing::warn!("process_leader_prepare(): {err:#}");
                            Err(())
                        }
                        Ok(()) => Ok(()),
                    };
                    metrics::ConsensusMsgLabel::LeaderPrepare.with_result(&res)
                }
                ConsensusMsg::LeaderCommit(_) => {
                    let res = match self
                        .process_leader_commit(ctx, signed_message.cast().unwrap())
                        .await
                        .wrap("process_leader_commit()")
                    {
                        Err(super::leader_commit::Error::Internal(err)) => return Err(err),
                        Err(err) => {
                            tracing::warn!("process_leader_commit(): {err:#}");
                            Err(())
                        }
                        Ok(()) => Ok(()),
                    };
                    metrics::ConsensusMsgLabel::LeaderCommit.with_result(&res)
                }
                _ => unreachable!(),
            };
            metrics::METRICS.replica_processing_latency[&label].observe_latency(ctx.now() - now);

            let _ = res_send.send(Ok(()));
        }
    }

    /// Backups the replica state to disk.
    pub(crate) async fn backup_state(&self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        let mut proposals = vec![];
        for (number, payloads) in &self.block_proposal_cache {
            proposals.extend(payloads.values().map(|p| storage::Proposal {
                number: *number,
                payload: p.clone(),
            }));
        }
        let backup = storage::ReplicaState {
            view: self.view,
            phase: self.phase,
            high_vote: self.high_vote,
            high_qc: self.high_qc.clone(),
            proposals,
        };
        self.config
            .replica_store
            .set_state(ctx, &backup)
            .await
            .wrap("put_replica_state")?;
        Ok(())
    }

    pub fn inbound_pruning_predicate(
        pending_msg: &Option<Signed<ConsensusMsg>>,
        new_msg: &Option<Signed<ConsensusMsg>>,
    ) -> bool {
        if pending_msg.is_none() || new_msg.is_none() {
            return false;
        }
        let (existing_msg, new_msg) = (pending_msg.as_ref().unwrap(), new_msg.as_ref().unwrap());

        if existing_msg.key != new_msg.key {
            return false;
        }

        match (&existing_msg.msg, &new_msg.msg) {
            (ConsensusMsg::LeaderPrepare(existing_msg), ConsensusMsg::LeaderPrepare(new_msg)) => {
               new_msg.view > existing_msg.view
            }
            (ConsensusMsg::LeaderCommit(existing_msg), ConsensusMsg::LeaderCommit(new_msg)) => {
                new_msg.justification.message.view > existing_msg.justification.message.view
            }
            _ => false,
        }
    }
}
