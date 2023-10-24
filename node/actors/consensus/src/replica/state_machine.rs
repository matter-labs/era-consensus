use crate::{metrics, ConsensusInner};
use concurrency::{ctx, metrics::LatencyHistogramExt as _, time};
use roles::validator;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use storage::Storage;
use tracing::{instrument, warn};

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
    pub(crate) storage: Arc<Storage>,
}

impl StateMachine {
    /// Creates a new StateMachine struct. We try to recover a past state from the storage module,
    /// otherwise we initialize the state machine with whatever head block we have.
    pub(crate) fn new(storage: Arc<Storage>) -> Self {
        match storage.get_replica_state() {
            Some(backup) => Self {
                view: backup.view,
                phase: backup.phase,
                high_vote: backup.high_vote,
                high_qc: backup.high_qc,
                block_proposal_cache: backup.block_proposal_cache,
                timeout_deadline: time::Deadline::Infinite,
                storage,
            },
            None => {
                let head = storage.get_head_block();

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
        }
    }

    /// Starts the state machine. The replica state needs to be initialized before
    /// we are able to process inputs. If we are in the genesis block, then we start a new view,
    /// this will kick start the consensus algorithm. Otherwise, we just start the timer.
    #[instrument(level = "trace", ret)]
    pub(crate) fn start(&mut self, ctx: &ctx::Ctx, consensus: &ConsensusInner) {
        if self.view == validator::ViewNumber(0) {
            self.start_new_view(ctx, consensus)
        } else {
            self.reset_timer(ctx)
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
    ) {
        let Some(signed_msg) = input else {
            warn!("We timed out before receiving a message.");
            // Start new view.
            self.start_new_view(ctx, consensus);
            return;
        };

        let now = ctx.now();
        let (label, result) = match &signed_msg.msg {
            validator::ConsensusMsg::LeaderPrepare(_) => (
                metrics::ConsensusMsgLabel::LeaderPrepare,
                self.process_leader_prepare(ctx, consensus, signed_msg.cast().unwrap()),
            ),
            validator::ConsensusMsg::LeaderCommit(_) => (
                metrics::ConsensusMsgLabel::LeaderCommit,
                self.process_leader_commit(ctx, consensus, signed_msg.cast().unwrap()),
            ),
            _ => unreachable!(),
        };
        metrics::METRICS.replica_processing_latency[&label.with_result(&result)]
            .observe_latency(ctx.now() - now);
        // All errors from processing inputs are recoverable, so we just log them.
        if let Err(e) = result {
            warn!("{}", e);
        }
    }

    /// Backups the replica state to disk.
    pub(crate) fn backup_state(&self) {
        let backup = storage::ReplicaState {
            view: self.view,
            phase: self.phase,
            high_vote: self.high_vote,
            high_qc: self.high_qc.clone(),
            block_proposal_cache: self.block_proposal_cache.clone(),
        };

        self.storage.put_replica_state(&backup);
    }
}
