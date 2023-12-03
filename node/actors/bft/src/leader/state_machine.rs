use crate::{metrics, ConsensusInner, PayloadSource};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    unreachable,
};
use tracing::instrument;
use zksync_concurrency::{ctx, metrics::LatencyHistogramExt as _, time, error::Wrap as _};
use zksync_consensus_roles::validator;

/// The StateMachine struct contains the state of the leader. This is a simple state machine. We just store
/// replica messages and produce leader messages (including proposing blocks) when we reach the threshold for
/// those messages. When participating in consensus we are not the leader most of the time.
pub(crate) struct StateMachine {
    /// Payload provider for the new blocks.
    pub(crate) payload_source: Arc<dyn PayloadSource>,
    /// The current view number. This might not match the replica's view number, we only have this here
    /// to make the leader advance monotonically in time and stop it from accepting messages from the past.
    pub(crate) view: validator::ViewNumber,
    /// The current phase. This might not match the replica's phase, we only have this here
    /// to make the leader advance monotonically in time and stop it from accepting messages from the past.
    pub(crate) phase: validator::Phase,
    /// Time when the current phase has started.
    pub(crate) phase_start: time::Instant,
    /// A cache of our latest block proposal. We use it to determine if we accept a replica commit message.
    pub(crate) block_proposal_cache: Option<validator::BlockHeader>,
    /// A cache of replica prepare messages indexed by view number and validator.
    pub(crate) prepare_message_cache: BTreeMap<
        validator::ViewNumber,
        HashMap<validator::PublicKey, validator::Signed<validator::ReplicaPrepare>>,
    >,
    /// A cache of replica commit messages indexed by view number and validator.
    pub(crate) commit_message_cache: BTreeMap<
        validator::ViewNumber,
        HashMap<validator::PublicKey, validator::Signed<validator::ReplicaCommit>>,
    >,
}

impl StateMachine {
    /// Creates a new StateMachine struct.
    #[instrument(level = "trace", skip(payload_source))]
    pub fn new(ctx: &ctx::Ctx, payload_source: Arc<dyn PayloadSource>) -> Self {
        StateMachine {
            payload_source,
            view: validator::ViewNumber(0),
            phase: validator::Phase::Prepare,
            phase_start: ctx.now(),
            block_proposal_cache: None,
            prepare_message_cache: BTreeMap::new(),
            commit_message_cache: BTreeMap::new(),
        }
    }

    /// Process an input message (leaders don't time out waiting for a message). This is the
    /// main entry point for the state machine. We need read-access to the inner consensus struct.
    /// As a result, we can modify our state machine or send a message to the executor.  
    #[instrument(level = "trace", skip(self), ret)]
    pub(crate) async fn process_input(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        input: validator::Signed<validator::ConsensusMsg>,
    ) -> ctx::Result<()> {
        let now = ctx.now();
        let label = match &input.msg {
            validator::ConsensusMsg::ReplicaPrepare(_) => {
                let res = match self
                    .process_replica_prepare(ctx, consensus, input.cast().unwrap())
                    .await.wrap("process_replica_prepare()")
                {
                    Ok(()) => Ok(()),
                    Err(super::replica_prepare::Error::Internal(err)) => {
                        return Err(err);
                    }
                    Err(err) => {
                        tracing::warn!("process_replica_prepare: {err:#}");
                        Err(())
                    }
                };
                metrics::ConsensusMsgLabel::ReplicaPrepare.with_result(&res)
            }
            validator::ConsensusMsg::ReplicaCommit(_) => {
                let res = self
                    .process_replica_commit(ctx, consensus, input.cast().unwrap())
                    .map_err(|err| {
                        tracing::warn!("process_replica_commit: {err:#}");
                    });
                metrics::ConsensusMsgLabel::ReplicaCommit.with_result(&res)
            }
            _ => unreachable!(),
        };
        metrics::METRICS.leader_processing_latency[&label].observe_latency(ctx.now() - now);
        Ok(())
    }
}
