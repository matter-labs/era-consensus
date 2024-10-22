use crate::{metrics, Config, OutputSender};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use zksync_concurrency::{
    ctx,
    error::Wrap as _,
    metrics::LatencyHistogramExt as _,
    sync::{self, prunable_mpsc::SelectionFunctionResult},
    time,
};
use zksync_consensus_network::io::ConsensusReq;
use zksync_consensus_roles::{validator, validator::ConsensusMsg};
use zksync_consensus_storage as storage;

/// The StateMachine struct contains the state of the replica. This is the most complex state machine and is responsible
/// for validating and voting on blocks. When participating in consensus we are always a replica.
#[derive(Debug)]
pub(crate) struct StateMachine {
    /// Consensus configuration and output channel.
    pub(crate) config: Arc<Config>,
    /// Pipe through which replica sends network messages.
    pub(super) outbound_pipe: OutputSender,
    /// Pipe through which replica receives network requests.
    inbound_pipe: sync::prunable_mpsc::Receiver<ConsensusReq>,
    /// The current view number.
    pub(crate) view: validator::ViewNumber,
    /// The current phase.
    pub(crate) phase: validator::Phase,
    /// The highest block proposal that the replica has committed to.
    pub(crate) high_vote: Option<validator::ReplicaCommit>,
    /// The highest commit quorum certificate known to the replica.
    pub(crate) high_commit_qc: Option<validator::CommitQC>,
    /// The highest timeout quorum certificate known to the replica.
    pub(crate) high_timeout_qc: Option<validator::TimeoutQC>,
    /// A cache of the received block proposals.
    pub(crate) block_proposal_cache:
        BTreeMap<validator::BlockNumber, HashMap<validator::PayloadHash, validator::Payload>>,
    /// The deadline to receive an input message.
    pub(crate) timeout_deadline: time::Deadline,
}

impl StateMachine {
    /// The duration of the timeout.
    pub(crate) const TIMEOUT_DURATION: time::Duration = time::Duration::milliseconds(2000);

    /// Creates a new [`StateMachine`] instance, attempting to recover a past state from the storage module,
    /// otherwise initializes the state machine with the current head block.
    ///
    /// Returns a tuple containing:
    /// * The newly created [`StateMachine`] instance.
    /// * A sender handle that should be used to send values to be processed by the instance, asynchronously.
    pub(crate) async fn start(
        ctx: &ctx::Ctx,
        config: Arc<Config>,
        outbound_pipe: OutputSender,
    ) -> ctx::Result<(Self, sync::prunable_mpsc::Sender<ConsensusReq>)> {
        let backup = config.replica_store.state(ctx).await?;

        let mut block_proposal_cache: BTreeMap<_, HashMap<_, _>> = BTreeMap::new();
        for proposal in backup.proposals {
            block_proposal_cache
                .entry(proposal.number)
                .or_default()
                .insert(proposal.payload.hash(), proposal.payload);
        }

        let (send, recv) = sync::prunable_mpsc::channel(
            StateMachine::inbound_filter_predicate,
            StateMachine::inbound_selection_function,
        );

        let mut this = Self {
            config,
            outbound_pipe,
            inbound_pipe: recv,
            view: backup.view,
            phase: backup.phase,
            high_vote: backup.high_vote,
            high_commit_qc: backup.high_commit_qc,
            high_timeout_qc: backup.high_timeout_qc,
            block_proposal_cache,
            timeout_deadline: time::Deadline::Infinite,
        };

        // We need to start the replica before processing inputs.
        this.start_new_view(ctx).await.wrap("start_new_view()")?;

        Ok((this, send))
    }

    /// Runs a loop to process incoming messages (may be `None` if the channel times out while waiting for a message).
    /// This is the main entry point for the state machine,
    /// potentially triggering state modifications and message sending to the executor.
    pub(crate) async fn run(mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        loop {
            let recv = self
                .inbound_pipe
                .recv(&ctx.with_deadline(self.timeout_deadline))
                .await;

            // Check for non-timeout cancellation.
            if !ctx.is_active() {
                return Ok(());
            }

            // Check for timeout.
            let Some(req) = recv.ok() else {
                self.start_new_view(ctx).await?;
                continue;
            };

            let now = ctx.now();
            let label = match &req.msg.msg {
                ConsensusMsg::ReplicaPrepare(_) => {
                    let res = match self
                        .process_replica_prepare(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("process_replica_prepare()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                super::replica_prepare::Error::Internal(e) => {
                                    tracing::error!(
                                        "process_replica_prepare: internal error: {e:#}"
                                    );
                                    return Err(e);
                                }
                                super::replica_prepare::Error::Old { .. } => {
                                    tracing::debug!("process_replica_prepare: {err:#}");
                                }
                                _ => {
                                    tracing::warn!("process_replica_prepare: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::ReplicaPrepare.with_result(&res)
                }
                ConsensusMsg::LeaderPrepare(_) => {
                    let res = match self
                        .on_proposal(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("process_leader_prepare()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                super::proposal::Error::Internal(e) => {
                                    tracing::error!(
                                        "process_leader_prepare: internal error: {e:#}"
                                    );
                                    return Err(e);
                                }
                                super::proposal::Error::Old { .. } => {
                                    tracing::info!("process_leader_prepare: {err:#}");
                                }
                                _ => {
                                    tracing::warn!("process_leader_prepare: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::LeaderPrepare.with_result(&res)
                }
                ConsensusMsg::LeaderCommit(_) => {
                    let res = match self
                        .process_leader_commit(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("process_leader_commit()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                super::leader_commit::Error::Internal(e) => {
                                    tracing::error!("process_leader_commit: internal error: {e:#}");
                                    return Err(e);
                                }
                                super::leader_commit::Error::Old { .. } => {
                                    tracing::info!("process_leader_commit: {err:#}");
                                }
                                _ => {
                                    tracing::warn!("process_leader_commit: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::LeaderCommit.with_result(&res)
                }
                _ => unreachable!(),
            };
            metrics::METRICS.replica_processing_latency[&label].observe_latency(ctx.now() - now);

            // Notify network actor that the message has been processed.
            // Ignore sending error.
            let _ = req.ack.send(());
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
            high_vote: self.high_vote.clone(),
            high_qc: self.high_commit_qc.clone(),
            proposals,
        };
        self.config
            .replica_store
            .set_state(ctx, &backup)
            .await
            .wrap("put_replica_state")?;
        Ok(())
    }

    fn inbound_filter_predicate(new_req: &ConsensusReq) -> bool {
        // Verify message signature
        new_req.msg.verify().is_ok()
    }

    fn inbound_selection_function(
        old_req: &ConsensusReq,
        new_req: &ConsensusReq,
    ) -> SelectionFunctionResult {
        if old_req.msg.key != new_req.msg.key {
            return SelectionFunctionResult::Keep;
        }

        match (&old_req.msg.msg, &new_req.msg.msg) {
            (ConsensusMsg::LeaderPrepare(old), ConsensusMsg::LeaderPrepare(new)) => {
                // Discard older message
                if old.view().number < new.view().number {
                    SelectionFunctionResult::DiscardOld
                } else {
                    SelectionFunctionResult::DiscardNew
                }
            }
            (ConsensusMsg::LeaderCommit(old), ConsensusMsg::LeaderCommit(new)) => {
                // Discard older message
                if old.view().number < new.view().number {
                    SelectionFunctionResult::DiscardOld
                } else {
                    SelectionFunctionResult::DiscardNew
                }
            }
            _ => SelectionFunctionResult::Keep,
        }
    }
}
