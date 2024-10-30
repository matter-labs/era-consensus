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
use zksync_consensus_roles::validator::{self, ConsensusMsg};

mod commit;
mod misc;
mod new_view;
mod proposal;
/// The proposer module contains the logic for the proposer role in ChonkyBFT.
pub(crate) mod proposer;
#[cfg(test)]
pub(crate) mod testonly;
#[cfg(test)]
mod tests;
mod timeout;

/// The StateMachine struct contains the state of the replica and implements all the
/// logic of ChonkyBFT.
#[derive(Debug)]
pub(crate) struct StateMachine {
    /// Consensus configuration.
    pub(crate) config: Arc<Config>,
    /// Pipe through which replica sends network messages.
    pub(super) outbound_pipe: OutputSender,
    /// Pipe through which replica receives network requests.
    pub(crate) inbound_pipe: sync::prunable_mpsc::Receiver<ConsensusReq>,
    /// The sender part of the proposer watch channel. This is used to notify the proposer loop
    /// and send the neeeded justification.
    pub(crate) proposer_pipe: sync::watch::Sender<Option<validator::ProposalJustification>>,

    /// The current view number.
    pub(crate) view_number: validator::ViewNumber,
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
    /// Latest view each validator has signed a ReplicaCommit message for.
    pub(crate) commit_views_cache: BTreeMap<validator::PublicKey, validator::ViewNumber>,
    /// Commit QCs indexed by view number and then by message.
    pub(crate) commit_qcs_cache:
        BTreeMap<validator::ViewNumber, BTreeMap<validator::ReplicaCommit, validator::CommitQC>>,
    /// Latest view each validator has signed a ReplicaTimeout message for.
    pub(crate) timeout_views_cache: BTreeMap<validator::PublicKey, validator::ViewNumber>,
    /// Timeout QCs indexed by view number.
    pub(crate) timeout_qcs_cache: BTreeMap<validator::ViewNumber, validator::TimeoutQC>,

    /// The deadline to receive an input message before timing out.
    pub(crate) timeout_deadline: time::Deadline,
    /// Time when the current phase has started. Used for metrics.
    pub(crate) phase_start: time::Instant,
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
        proposer_pipe: sync::watch::Sender<Option<validator::ProposalJustification>>,
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

        let this = Self {
            config,
            outbound_pipe,
            inbound_pipe: recv,
            proposer_pipe,
            view_number: backup.view,
            phase: backup.phase,
            high_vote: backup.high_vote,
            high_commit_qc: backup.high_commit_qc,
            high_timeout_qc: backup.high_timeout_qc,
            block_proposal_cache,
            commit_views_cache: BTreeMap::new(),
            commit_qcs_cache: BTreeMap::new(),
            timeout_views_cache: BTreeMap::new(),
            timeout_qcs_cache: BTreeMap::new(),
            timeout_deadline: time::Deadline::Finite(ctx.now() + Self::TIMEOUT_DURATION),
            phase_start: ctx.now(),
        };

        Ok((this, send))
    }

    /// Runs a loop to process incoming messages (may be `None` if the channel times out while waiting for a message).
    /// This is the main entry point for the state machine,
    /// potentially triggering state modifications and message sending to the executor.
    pub(crate) async fn run(mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        // If this is the first view, we immediately timeout. This will force the replicas
        // to synchronize right at the beginning and will provide a justification for the
        // next view. This is necessary because the first view is not justified by any
        // previous view.
        if self.view_number == validator::ViewNumber(0) {
            self.start_timeout(ctx).await?;
        }

        // Main loop.
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
                self.start_timeout(ctx).await?;
                continue;
            };

            let now = ctx.now();
            let label = match &req.msg.msg {
                ConsensusMsg::LeaderProposal(_) => {
                    let res = match self
                        .on_proposal(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("on_proposal()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                // If the error is internal, we stop here.
                                proposal::Error::Internal(e) => {
                                    tracing::error!("on_proposal: internal error: {e:#}");
                                    return Err(e);
                                }
                                // If the error is due to an old message, we log it at a lower level.
                                proposal::Error::Old { .. } => {
                                    tracing::debug!("on_proposal: {err:#}");
                                }
                                _ => {
                                    tracing::warn!("on_proposal: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::ReplicaPrepare.with_result(&res)
                }
                ConsensusMsg::ReplicaCommit(_) => {
                    let res = match self
                        .on_commit(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("on_commit()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                // If the error is internal, we stop here.
                                commit::Error::Internal(e) => {
                                    tracing::error!("on_commit: internal error: {e:#}");
                                    return Err(e);
                                }
                                // If the error is due to an old message, we log it at a lower level.
                                commit::Error::Old { .. } => {
                                    tracing::debug!("on_commit: {err:#}");
                                }
                                _ => {
                                    tracing::warn!("on_commit: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::ReplicaPrepare.with_result(&res)
                }
                ConsensusMsg::ReplicaTimeout(_) => {
                    let res = match self
                        .on_timeout(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("on_timeout()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                // If the error is internal, we stop here.
                                timeout::Error::Internal(e) => {
                                    tracing::error!("on_timeout: internal error: {e:#}");
                                    return Err(e);
                                }
                                // If the error is due to an old message, we log it at a lower level.
                                timeout::Error::Old { .. } => {
                                    tracing::debug!("on_timeout: {err:#}");
                                }
                                _ => {
                                    tracing::warn!("on_timeout: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::ReplicaPrepare.with_result(&res)
                }
                ConsensusMsg::ReplicaNewView(_) => {
                    let res = match self
                        .on_new_view(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("on_new_view()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                // If the error is internal, we stop here.
                                new_view::Error::Internal(e) => {
                                    tracing::error!("on_new_view: internal error: {e:#}");
                                    return Err(e);
                                }
                                // If the error is due to an old message, we log it at a lower level.
                                new_view::Error::Old { .. } => {
                                    tracing::debug!("on_new_view: {err:#}");
                                }
                                _ => {
                                    tracing::warn!("on_new_view: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::ReplicaPrepare.with_result(&res)
                }
            };
            metrics::METRICS.replica_processing_latency[&label].observe_latency(ctx.now() - now);

            // Notify network actor that the message has been processed.
            // Ignore sending error.
            let _ = req.ack.send(());
        }
    }

    fn inbound_filter_predicate(new_req: &ConsensusReq) -> bool {
        // Verify message signature
        new_req.msg.verify().is_ok()
    }

    fn inbound_selection_function(
        old_req: &ConsensusReq,
        new_req: &ConsensusReq,
    ) -> SelectionFunctionResult {
        if old_req.msg.key != new_req.msg.key || old_req.msg.msg.label() != new_req.msg.msg.label()
        {
            SelectionFunctionResult::Keep
        } else {
            // Discard older message
            if old_req.msg.msg.view().number < new_req.msg.msg.view().number {
                SelectionFunctionResult::DiscardOld
            } else {
                SelectionFunctionResult::DiscardNew
            }
        }
    }
}
