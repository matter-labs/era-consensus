use crate::{metrics, Config, OutputPipe};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use tracing::instrument;
use zksync_concurrency::{ctx, error::Wrap as _, metrics::LatencyHistogramExt as _, time};
use zksync_consensus_roles::validator;
use zksync_consensus_storage as storage;

/// The StateMachine struct contains the state of the replica. This is the most complex state machine and is responsible
/// for validating and voting on blocks. When participating in consensus we are always a replica.
#[derive(Debug)]
pub(crate) struct StateMachine {
    /// Consensus configuration and output channel.
    pub(crate) config: Arc<Config>,
    /// Pipe through with replica sends network messages.
    pub(super) pipe: OutputPipe,
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
}

impl StateMachine {
    /// Creates a new StateMachine struct. We try to recover a past state from the storage module,
    /// otherwise we initialize the state machine with whatever head block we have.
    pub(crate) async fn start(
        ctx: &ctx::Ctx,
        config: Arc<Config>,
        pipe: OutputPipe,
    ) -> ctx::Result<Self> {
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

        let mut this = Self {
            config,
            pipe,
            view: backup.view,
            phase: backup.phase,
            high_vote: backup.high_vote,
            high_qc: backup.high_qc,
            block_proposal_cache,
            timeout_deadline: time::Deadline::Infinite,
        };
        // We need to start the replica before processing inputs.
        this.start_new_view(ctx).await.wrap("start_new_view()")?;
        Ok(this)
    }

    /// Process an input message (it will be None if the channel timed out waiting for a message). This is
    /// the main entry point for the state machine. We need read-access to the inner consensus struct.
    /// As a result, we can modify our state machine or send a message to the executor.
    #[instrument(level = "trace", ret)]
    pub(crate) async fn process_input(
        &mut self,
        ctx: &ctx::Ctx,
        input: validator::Signed<validator::ConsensusMsg>,
    ) -> ctx::Result<()> {
        let now = ctx.now();
        let label = match &input.msg {
            validator::ConsensusMsg::LeaderPrepare(_) => {
                let res = match self
                    .process_leader_prepare(ctx, input.cast().unwrap())
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
                    .process_leader_commit(ctx, input.cast().unwrap())
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
}
