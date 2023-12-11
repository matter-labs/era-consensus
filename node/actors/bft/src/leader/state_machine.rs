use crate::{metrics, ConsensusInner, PayloadSource};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    unreachable,
};
use tracing::instrument;
use zksync_concurrency::{ctx, sync, error::Wrap as _, metrics::LatencyHistogramExt as _, time};
use zksync_consensus_roles::validator;
use zksync_consensus_network::io::{ConsensusInputMessage, Target};

/// The StateMachine struct contains the state of the leader. This is a simple state machine. We just store
/// replica messages and produce leader messages (including proposing blocks) when we reach the threshold for
/// those messages. When participating in consensus we are not the leader most of the time.
pub(crate) struct StateMachine {
    pub(crate) inner: Arc<ConsensusInner>,
    /// The current view number. This might not match the replica's view number, we only have this here
    /// to make the leader advance monotonically in time and stop it from accepting messages from the past.
    pub(crate) view: validator::ViewNumber,
    /// The current phase. This might not match the replica's phase, we only have this here
    /// to make the leader advance monotonically in time and stop it from accepting messages from the past.
    pub(crate) phase: validator::Phase,
    /// Time when the current phase has started.
    pub(crate) phase_start: time::Instant,
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
    /// Newest quorum certificate composed from the `ReplicaPrepare` messages.
    pub(crate) prepare_qc: sync::watch::Sender<Option<validator::PrepareQC>>,
}

impl StateMachine {
    /// Creates a new StateMachine struct.
    #[instrument(level = "trace")]
    pub fn new(ctx: &ctx::Ctx, inner: Arc<ConsensusInner>) -> Self {
        StateMachine {
            inner,
            view: validator::ViewNumber(0),
            phase: validator::Phase::Prepare,
            phase_start: ctx.now(),
            prepare_message_cache: BTreeMap::new(),
            commit_message_cache: BTreeMap::new(),
            prepare_qc: sync::watch::channel(None).0,
        }
    }

    /// Process an input message (leaders don't time out waiting for a message). This is the
    /// main entry point for the state machine. We need read-access to the inner consensus struct.
    /// As a result, we can modify our state machine or send a message to the executor.  
    #[instrument(level = "trace", skip(self), ret)]
    pub(crate) async fn process_input(
        &mut self,
        ctx: &ctx::Ctx,
        input: validator::Signed<validator::ConsensusMsg>,
    ) -> ctx::Result<()> {
        let now = ctx.now();
        let label = match &input.msg {
            validator::ConsensusMsg::ReplicaPrepare(_) => {
                let res = match self
                    .process_replica_prepare(ctx, input.cast().unwrap())
                    .await
                    .wrap("process_replica_prepare()")
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
                    .process_replica_commit(ctx, input.cast().unwrap())
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

    pub async fn run_proposer(
        ctx: &ctx::Ctx,
        inner: &ConsensusInner,
        payload_source: &dyn PayloadSource,
        mut prepare_qc: sync::watch::Receiver<Option<validator::PrepareQC>>,
    ) -> ctx::Result<()> {
        let mut next_view = validator::ViewNumber(0);
        loop {
            sync::changed(ctx, &mut prepare_qc).await?;
            let Some(prepare_qc) = prepare_qc.borrow_and_update().clone() else { continue };
            if next_view < prepare_qc.view() { continue };
            next_view = prepare_qc.view();
            let msg = Self::propose(ctx,inner,payload_source, prepare_qc).await?; 
        }
    }

    pub(crate) async fn propose(
        ctx: &ctx::Ctx,
        inner: &ConsensusInner,
        payload_source: &dyn PayloadSource,
        justification: validator::PrepareQC,
    ) -> ctx::Result<()> {
        // Get the highest block voted for and check if there's a quorum of votes for it. To have a quorum
        // in this situation, we require 2*f+1 votes, where f is the maximum number of faulty replicas.
        let mut count: HashMap<_, usize> = HashMap::new();

        for (vote,signers) in justification.map.iter() {
            *count.entry(vote.high_vote.proposal).or_default() += signers.len();
        }

        let highest_vote: Option<validator::BlockHeader> = count
            .iter()
            // We only take one value from the iterator because there can only be at most one block with a quorum of 2f+1 votes.
            .find(|(_, v)| **v > 2 * inner.faulty_replicas())
            .map(|(h, _)| h)
            .cloned();

        // Get the highest CommitQC.
        let highest_qc: &validator::CommitQC = justification.map.keys()
            .map(|s| &s.high_qc)
            .max_by_key(|qc| qc.message.view)
            .unwrap();

        // Create the block proposal to send to the replicas,
        // and the commit vote to store in our block proposal cache.
        let (proposal, payload) = match highest_vote {
            // The previous block was not finalized, so we need to propose it again.
            // For this we only need the header, since we are guaranteed that at least
            // f+1 honest replicas have the block and can broadcast it when finalized
            // (2f+1 have stated that they voted for the block, at most f are malicious).
            Some(proposal) if proposal != highest_qc.message.proposal => (proposal, None),
            // The previous block was finalized, so we can propose a new block.
            _ => {
                let payload = payload_source
                    .propose(ctx, highest_qc.message.proposal.number.next())
                    .await?;
                metrics::METRICS
                    .leader_proposal_payload_size
                    .observe(payload.0.len());
                let proposal =
                    validator::BlockHeader::new(&highest_qc.message.proposal, payload.hash());
                (proposal, Some(payload))
            }
        };

        // ----------- Prepare our message and send it --------------

        // Broadcast the leader prepare message to all replicas (ourselves included).
        let msg = inner
            .secret_key
            .sign_msg(validator::ConsensusMsg::LeaderPrepare(
                validator::LeaderPrepare {
                    protocol_version: crate::PROTOCOL_VERSION,
                    view: justification.view(),
                    proposal,
                    proposal_payload: payload,
                    justification,
                },
            ));
        inner.pipe.send(ConsensusInputMessage {
            message: msg.embed().into(),
            recipient: Target::Broadcast,
        });
        Ok(())
    }
}
