use crate::{metrics, Config, OutputSender};
use std::{collections::BTreeMap, sync::Arc, unreachable};
use tracing::instrument;
use zksync_concurrency::{
    ctx,
    error::Wrap as _,
    metrics::LatencyHistogramExt as _,
    sync::{self, prunable_mpsc::SelectionFunctionResult},
    time,
};
use zksync_consensus_network::io::{ConsensusInputMessage, ConsensusReq, Target};
use zksync_consensus_roles::validator;

/// The StateMachine struct contains the state of the leader. This is a simple state machine. We just store
/// replica messages and produce leader messages (including proposing blocks) when we reach the threshold for
/// those messages. When participating in consensus we are not the leader most of the time.
pub(crate) struct StateMachine {
    /// Consensus configuration and output channel.
    pub(crate) config: Arc<Config>,
    /// Pipe through which leader sends network messages.
    pub(crate) outbound_pipe: OutputSender,
    /// Pipe through which leader receives network requests.
    pub(crate) inbound_pipe: sync::prunable_mpsc::Receiver<ConsensusReq>,
    /// The current view number. This might not match the replica's view number, we only have this here
    /// to make the leader advance monotonically in time and stop it from accepting messages from the past.
    pub(crate) view: validator::ViewNumber,
    /// The current phase. This might not match the replica's phase, we only have this here
    /// to make the leader advance monotonically in time and stop it from accepting messages from the past.
    pub(crate) phase: validator::Phase,
    /// Time when the current phase has started.
    pub(crate) phase_start: time::Instant,
    /// Latest view each validator has signed a ReplicaPrepare message for.
    pub(crate) replica_prepare_views: BTreeMap<validator::PublicKey, validator::ViewNumber>,
    /// Prepare QCs indexed by view number.
    pub(crate) prepare_qcs: BTreeMap<validator::ViewNumber, validator::PrepareQC>,
    /// Newest prepare QC composed from the `ReplicaPrepare` messages.
    pub(crate) prepare_qc: sync::watch::Sender<Option<validator::PrepareQC>>,
    /// Commit QCs indexed by view number and then by message.
    pub(crate) commit_qcs:
        BTreeMap<validator::ViewNumber, BTreeMap<validator::ReplicaCommit, validator::CommitQC>>,
    /// Latest view each validator has signed a ReplicaCommit message for.
    pub(crate) replica_commit_views: BTreeMap<validator::PublicKey, validator::ViewNumber>,
}

impl StateMachine {
    /// Creates a new [`StateMachine`] instance.
    ///
    /// Returns a tuple containing:
    /// * The newly created [`StateMachine`] instance.
    /// * A sender handle that should be used to send values to be processed by the instance, asynchronously.
    #[instrument(level = "trace")]
    pub fn new(
        ctx: &ctx::Ctx,
        config: Arc<Config>,
        outbound_pipe: OutputSender,
    ) -> (Self, sync::prunable_mpsc::Sender<ConsensusReq>) {
        let (send, recv) = sync::prunable_mpsc::channel(
            StateMachine::inbound_filter_predicate,
            StateMachine::inbound_selection_function,
        );

        let this = StateMachine {
            config,
            outbound_pipe,
            view: validator::ViewNumber(0),
            phase: validator::Phase::Prepare,
            phase_start: ctx.now(),
            replica_prepare_views: BTreeMap::new(),
            prepare_qcs: BTreeMap::new(),
            prepare_qc: sync::watch::channel(None).0,
            commit_qcs: BTreeMap::new(),
            inbound_pipe: recv,
            replica_commit_views: BTreeMap::new(),
        };

        (this, send)
    }

    /// Runs a loop to process incoming messages.
    /// This is the main entry point for the state machine,
    /// potentially triggering state modifications and message sending to the executor.
    pub(crate) async fn run(mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        loop {
            let req = self.inbound_pipe.recv(ctx).await?;

            let now = ctx.now();
            use validator::ConsensusMsg as M;
            let label = match &req.msg.msg {
                M::ReplicaPrepare(_) => {
                    let res = match self
                        .process_replica_prepare(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("process_replica_prepare()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                super::replica_prepare::Error::Internal(e) => {
                                    return Err(e);
                                }
                                super::replica_prepare::Error::Old { .. }
                                | super::replica_prepare::Error::NotLeaderInView => {
                                    tracing::info!("process_replica_prepare: {err:#}");
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
                M::ReplicaCommit(_) => {
                    let res = self
                        .process_replica_commit(ctx, req.msg.cast().unwrap())
                        .map_err(|err| {
                            tracing::warn!("process_replica_commit: {err:#}");
                        });
                    metrics::ConsensusMsgLabel::ReplicaCommit.with_result(&res)
                }
                _ => unreachable!(),
            };
            metrics::METRICS.leader_processing_latency[&label].observe_latency(ctx.now() - now);

            // Notify network actor that the message has been processed.
            // Ignore sending error.
            let _ = req.ack.send(());
        }
    }

    /// In a loop, receives a PrepareQC and sends a LeaderPrepare containing it.
    /// Every subsequent PrepareQC has to be for a higher view than the previous one (otherwise it
    /// is skipped). In case payload generation takes too long, some PrepareQC may be elided, so
    /// that the validator doesn't spend time on generating payloads for already expired views.
    pub(crate) async fn run_proposer(
        ctx: &ctx::Ctx,
        config: &Config,
        mut prepare_qc: sync::watch::Receiver<Option<validator::PrepareQC>>,
        pipe: &OutputSender,
    ) -> ctx::Result<()> {
        let mut next_view = validator::ViewNumber(0);
        loop {
            let Some(prepare_qc) = sync::changed(ctx, &mut prepare_qc).await?.clone() else {
                continue;
            };
            if prepare_qc.view.number < next_view {
                continue;
            };
            next_view = prepare_qc.view.number.next();
            Self::propose(ctx, config, prepare_qc, pipe).await?;
        }
    }

    /// Sends a LeaderPrepare for the given PrepareQC.
    /// Uses `payload_source` to generate a payload if needed.
    pub(crate) async fn propose(
        ctx: &ctx::Ctx,
        cfg: &Config,
        justification: validator::PrepareQC,
        pipe: &OutputSender,
    ) -> ctx::Result<()> {
        let high_vote = justification.high_vote(cfg.genesis());
        let high_qc = justification.high_qc();

        // Create the block proposal to send to the replicas,
        // and the commit vote to store in our block proposal cache.
        let (proposal, payload) = match high_vote {
            // The previous block was not finalized, so we need to propose it again.
            // For this we only need the header, since we are guaranteed that at least
            // f+1 honest replicas have the block and can broadcast it when finalized
            // (2f+1 have stated that they voted for the block, at most f are malicious).
            Some(proposal) if Some(&proposal) != high_qc.map(|qc| &qc.message.proposal) => {
                (proposal, None)
            }
            // The previous block was finalized, so we can propose a new block.
            _ => {
                let number = match high_qc {
                    Some(qc) => qc.header().number.next(),
                    None => cfg.genesis().first_block,
                };
                // Defensively assume that PayloadManager cannot propose until the previous block is stored.
                if let Some(prev) = number.prev() {
                    cfg.block_store
                        .wait_until_block_persisted(ctx, prev)
                        .await?;
                }
                let payload = cfg.payload_manager.propose(ctx, number).await?;
                if payload.0.len() > cfg.max_payload_size {
                    return Err(anyhow::format_err!(
                        "proposed payload too large: got {}B, max {}B",
                        payload.0.len(),
                        cfg.max_payload_size
                    )
                    .into());
                }
                metrics::METRICS
                    .leader_proposal_payload_size
                    .observe(payload.0.len());
                let proposal = validator::BlockHeader {
                    number,
                    payload: payload.hash(),
                };
                (proposal, Some(payload))
            }
        };

        // ----------- Prepare our message and send it --------------

        // Broadcast the leader prepare message to all replicas (ourselves included).
        let msg = cfg
            .secret_key
            .sign_msg(validator::ConsensusMsg::LeaderPrepare(
                validator::LeaderPrepare {
                    proposal,
                    proposal_payload: payload,
                    justification,
                },
            ));
        pipe.send(
            ConsensusInputMessage {
                message: msg,
                recipient: Target::Broadcast,
            }
            .into(),
        );
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
        use validator::ConsensusMsg as M;
        match (&old_req.msg.msg, &new_req.msg.msg) {
            (M::ReplicaPrepare(old), M::ReplicaPrepare(new)) => {
                // Discard older message
                if old.view.number < new.view.number {
                    SelectionFunctionResult::DiscardOld
                } else {
                    SelectionFunctionResult::DiscardNew
                }
            }
            (M::ReplicaCommit(old), M::ReplicaCommit(new)) => {
                // Discard older message
                if old.view.number < new.view.number {
                    SelectionFunctionResult::DiscardOld
                } else {
                    SelectionFunctionResult::DiscardNew
                }
            }
            _ => SelectionFunctionResult::Keep,
        }
    }
}
