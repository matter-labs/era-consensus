use crate::{metrics, ConsensusInner};
use std::collections::{BTreeMap, HashMap};
use tracing::instrument;
use zksync_concurrency::{ctx, error::Wrap as _, metrics::LatencyHistogramExt as _, time};
use zksync_consensus_roles::validator;
use zksync_consensus_storage as storage;
use zksync_consensus_storage::ReplicaStore;

/// The StateMachine struct contains the state of the replica. This is the most complex state machine and is responsible
/// for validating and voting on blocks. When participating in consensus we are always a replica.
#[derive(Debug)]
pub(crate) struct StateMachine {
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
    pub(crate) timeout_deadline: time::Deadline,
    /// A reference to the storage module. We use it to backup the replica state and store
    /// finalized blocks.
    pub(crate) storage: ReplicaStore,
}

impl StateMachine {
    /// Creates a new StateMachine struct. We try to recover a past state from the storage module,
    /// otherwise we initialize the state machine with whatever head block we have.
    pub(crate) async fn new(ctx: &ctx::Ctx, storage: ReplicaStore) -> anyhow::Result<Self> {
        let backup = storage.replica_state(ctx).await?;
        let mut block_proposal_cache: BTreeMap<_, HashMap<_, _>> = BTreeMap::new();
        for proposal in backup.proposals {
            block_proposal_cache
                .entry(proposal.number)
                .or_default()
                .insert(proposal.payload.hash(), proposal.payload);
        }

        Ok(Self {
            view: backup.view,
            phase: backup.phase,
            high_vote: backup.high_vote,
            high_qc: backup.high_qc,
            block_proposal_cache,
            timeout_deadline: time::Deadline::Infinite,
            storage,
        })
    }

    /// Starts the state machine. The replica state needs to be initialized before
    /// we are able to process inputs. If we are in the genesis block, then we start a new view,
    /// this will kick start the consensus algorithm. Otherwise, we just start the timer.
    #[instrument(level = "trace", ret)]
    pub(crate) async fn start(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
    ) -> Result<(), ctx::Error> {
        if self.view == validator::ViewNumber(0) {
            self.start_new_view(ctx, consensus)
                .await
                .wrap("start_new_view()")
        } else {
            self.reset_timer(ctx);
            Ok(())
        }
    }

    /// Process an input message (it will be None if the channel timed out waiting for a message). This is
    /// the main entry point for the state machine. We need read-access to the inner consensus struct.
    /// As a result, we can modify our state machine or send a message to the executor.
    #[instrument(level = "trace", ret)]
    pub(crate) async fn process_input(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        input: Option<validator::Signed<validator::ConsensusMsg>>,
    ) -> Result<(), ctx::Error> {
        let Some(signed_msg) = input else {
            tracing::warn!("We timed out before receiving a message.");
            // Start new view.
            self.start_new_view(ctx, consensus).await?;
            return Ok(());
        };

        let now = ctx.now();
        let label = match &signed_msg.msg {
            validator::ConsensusMsg::LeaderPrepare(_) => {
                let res = match self
                    .process_leader_prepare(ctx, consensus, signed_msg.cast().unwrap())
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
            validator::ConsensusMsg::LeaderCommit(_) => {
                let res = match self
                    .process_leader_commit(ctx, consensus, signed_msg.cast().unwrap())
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
        Ok(())
    }

    /// Backups the replica state to disk.
    pub(crate) async fn backup_state(&self, ctx: &ctx::Ctx) -> Result<(), ctx::Error> {
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
        self.storage
            .put_replica_state(ctx, &backup)
            .await
            .wrap("put_replica_state")?;
        Ok(())
    }
}
