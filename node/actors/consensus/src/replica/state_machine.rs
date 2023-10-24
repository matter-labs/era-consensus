use crate::{metrics, replica::error::Error, ConsensusInner};
use concurrency::{ctx, metrics::LatencyHistogramExt as _, scope, time};
use roles::validator;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use storage::{ReplicaStateStore, StorageError};
use tracing::instrument;

#[derive(thiserror::Error, Debug)]
pub(crate) enum Error {
    #[error("LeaderPrepare: {0}")]
    LeaderPrepare(#[from] super::leader_prepare::Error),
    #[error("LeaderCommit: {0}")]
    LeaderCommit(#[from] super::leader_commit::Error),
}

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
    /// A reference to the storage module. We use it to backup the replica state.
    pub(crate) storage: Arc<dyn ReplicaStateStore>,
}

impl StateMachine {
    /// Creates a new StateMachine struct. We try to recover a past state from the storage module,
    /// otherwise we initialize the state machine with whatever head block we have.
    pub(crate) async fn new(
        ctx: &ctx::Ctx,
        storage: Arc<dyn ReplicaStateStore>,
    ) -> anyhow::Result<Self> {
        Ok(match storage.replica_state(ctx).await? {
            Some(backup) => {
                let mut block_proposal_cache : BTreeMap<_, HashMap<_,_>> = BTreeMap::new();
                for p in backup.proposals {
                    block_proposal_cache.entry(p.number).or_default().insert(p.payload.hash(),p.payload);
                }
                Self {
                    view: backup.view,
                    phase: backup.phase,
                    high_vote: backup.high_vote,
                    high_qc: backup.high_qc,
                    block_proposal_cache: backup.block_proposal_cache,
                    timeout_deadline: time::Deadline::Infinite,
                    storage,
                }
            },
            None => {
                let head = storage.head_block(ctx).await?;
                Self {
                    view: head.justification.message.view,
                    phase: validator::Phase::Prepare,
                    high_vote: head.justification.message,
                    high_qc: head.justification,
                    block_proposal_cache: BTreeMap::new(),
                    timeout_deadline: time::Deadline::Infinite,
                    storage,
                }
            }
        })
    }

    /// Starts the state machine. The replica state needs to be initialized before
    /// we are able to process inputs. If we are in the genesis block, then we start a new view,
    /// this will kick start the consensus algorithm. Otherwise, we just start the timer.
    #[instrument(level = "trace", ret)]
    pub(crate) fn start(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
    ) -> Result<(), Error> {
        if self.view == validator::ViewNumber(0) {
            self.start_new_view(ctx, consensus)
        } else {
            self.reset_timer(ctx);
            Ok(())
        }
    }

    /// Process an input message (it will be None if the channel timed out waiting for a message). This is
    /// the main entry point for the state machine. We need read-access to the inner consensus struct.
    /// As a result, we can modify our state machine or send a message to the executor.
    #[instrument(level = "trace", ret)]
    pub(crate) fn process_input(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        input: Option<validator::Signed<validator::ConsensusMsg>>,
    ) -> anyhow::Result<()> {
        let Some(signed_msg) = input else {
            tracing::warn!("We timed out before receiving a message.");
            // Start new view.
            self.start_new_view(ctx, consensus)?;
            return Ok(());
        };

        let now = ctx.now();
        let (label, result) = match &signed_msg.msg {
            validator::ConsensusMsg::LeaderPrepare(_) => (
                metrics::ConsensusMsgLabel::LeaderPrepare,
                self.process_leader_prepare(ctx, consensus, signed_msg.cast().unwrap()).map_err(Error::LeaderPrepare),
            ),
            validator::ConsensusMsg::LeaderCommit(_) => (
                metrics::ConsensusMsgLabel::LeaderCommit,
                self.process_leader_commit(ctx, consensus, signed_msg.cast().unwrap()).map_err(Error::LeaderCommit),
            ),
            _ => unreachable!(),
        };
        metrics::METRICS.replica_processing_latency[&label.with_result(&result)]
            .observe_latency(ctx.now() - now);
        match result {
            Ok(()) => Ok(()),
            Err(err @ Error::ReplicaStateSave(_)) => Err(err.into()),
            Err(err) => {
                // Other errors from processing inputs are recoverable, so we just log them.
                tracing::warn!("{err}");
                Ok(())
            }
        }
    }

    /// Backups the replica state to disk.
    pub(crate) fn backup_state(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let mut proposals = vec![];
        for (number,payloads) in &self.block_proposal_cache {
            proposals.extend(payloads.values().map(|p|storage::Proposal { number: *number, payload: p.clone() }));
        }
        let backup = storage::ReplicaState {
            view: self.view,
            phase: self.phase,
            high_vote: self.high_vote,
            high_qc: self.high_qc.clone(),
            proposals,
        };

        let store_result = scope::run_blocking!(ctx, |ctx, s| {
            let backup_future = self.storage.put_replica_state(ctx, &backup);
            s.spawn(backup_future).join(ctx).block()?;
            Ok(())
        });
        match store_result {
            Ok(()) => { /* Everything went fine */ }
            Err(StorageError::Canceled(_)) => tracing::trace!("Storing replica state was canceled"),
            Err(StorageError::Database(err)) => return Err(err),
        }
        Ok(())
    }
}
