//! The ChonkyBFT module contains the implementation of the ChonkyBFT consensus protocol.
//! This corresponds to the version 2 of the protocol.
//! The module is responsible for handling the logic that allows us to reach agreement on blocks.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use zksync_concurrency::{ctx, error::Wrap as _, metrics::LatencyHistogramExt as _, sync, time};
use zksync_consensus_roles::validator;

use crate::{metrics, Config, FromNetworkMessage, ToNetworkMessage};

mod block;
mod commit;
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
    /// Channel through which replica sends network messages.
    pub(super) outbound_channel: ctx::channel::UnboundedSender<ToNetworkMessage>,
    /// Channel through which replica receives network requests.
    pub(crate) inbound_channel: sync::prunable_mpsc::Receiver<FromNetworkMessage>,
    /// The sender part of the proposer watch channel. This is used to notify the proposer loop
    /// and send the needed justification.
    pub(crate) proposer_sender: sync::watch::Sender<Option<validator::v2::ProposalJustification>>,

    /// The current view number.
    pub(crate) view_number: validator::ViewNumber,
    /// The current phase.
    pub(crate) phase: validator::v2::Phase,
    /// The highest block proposal that the replica has committed to.
    pub(crate) high_vote: Option<validator::v2::ReplicaCommit>,
    /// The highest commit quorum certificate known to the replica.
    pub(crate) high_commit_qc: Option<validator::v2::CommitQC>,
    /// The highest timeout quorum certificate known to the replica.
    pub(crate) high_timeout_qc: Option<validator::v2::TimeoutQC>,

    /// A cache of the received block proposals.
    pub(crate) block_proposal_cache:
        BTreeMap<validator::BlockNumber, HashMap<validator::PayloadHash, validator::Payload>>,
    /// Latest view each validator has signed a ReplicaCommit message for.
    pub(crate) commit_views_cache: BTreeMap<validator::PublicKey, validator::ViewNumber>,
    /// Commit QCs indexed by view number and then by message.
    pub(crate) commit_qcs_cache: BTreeMap<
        validator::ViewNumber,
        BTreeMap<validator::v2::ReplicaCommit, validator::v2::CommitQC>,
    >,
    /// Latest view each validator has signed a ReplicaTimeout message for.
    pub(crate) timeout_views_cache: BTreeMap<validator::PublicKey, validator::ViewNumber>,
    /// Timeout QCs indexed by view number.
    pub(crate) timeout_qcs_cache: BTreeMap<validator::ViewNumber, validator::v2::TimeoutQC>,

    /// The deadline to receive a proposal for this view before timing out.
    pub(crate) view_timeout: time::Deadline,
    /// Time when the current view phase has started. Used for metrics.
    pub(crate) view_start: time::Instant,
}

impl StateMachine {
    /// Creates a new [`StateMachine`] instance, attempting to recover a past state from the storage module,
    /// otherwise initializes the state machine with the current head block.
    ///
    /// Returns a tuple containing:
    /// * The newly created [`StateMachine`] instance.
    /// * A sender handle that should be used to send values to be processed by the instance, asynchronously.
    pub(crate) async fn start(
        ctx: &ctx::Ctx,
        config: Arc<Config>,
        outbound_channel: ctx::channel::UnboundedSender<ToNetworkMessage>,
        inbound_channel: sync::prunable_mpsc::Receiver<FromNetworkMessage>,
        proposer_sender: sync::watch::Sender<Option<validator::v2::ProposalJustification>>,
    ) -> ctx::Result<Self> {
        let backup_opt = match config.engine_manager.get_state(ctx).await? {
            validator::ReplicaState::V2(backup) => Some(backup),
        };

        let backup = match backup_opt {
            Some(backup) => {
                if backup.epoch == config.epoch {
                    // If the backup epoch matches the current epoch, we return the existing backup state.
                    tracing::trace!(
                        "ChonkyBFT replica - Starting from backup state for epoch {}.",
                        backup.epoch
                    );
                    backup
                } else {
                    // If the backup epoch does not match the current epoch, we return a default state.
                    // This will cause the replica to start from the beginning of the current epoch.
                    tracing::trace!(
                        "ChonkyBFT replica - Backup epoch {} does not match current epoch {}",
                        backup.epoch,
                        config.epoch
                    );
                    validator::v2::ChonkyV2State::default()
                }
            }
            None => {
                // If there is no backup state, we return a default state.
                // This will cause the replica to start from the beginning of the current epoch.
                tracing::trace!("ChonkyBFT replica - No backup state found.");
                validator::v2::ChonkyV2State::default()
            }
        };

        let mut block_proposal_cache: BTreeMap<_, HashMap<_, _>> = BTreeMap::new();
        for proposal in backup.proposals {
            block_proposal_cache
                .entry(proposal.number)
                .or_default()
                .insert(proposal.payload.hash(), proposal.payload);
        }

        let this = Self {
            view_timeout: time::Deadline::Finite(ctx.now() + config.view_timeout),
            config,
            outbound_channel,
            inbound_channel,
            proposer_sender,
            view_number: backup.view_number,
            phase: backup.phase,
            high_vote: backup.high_vote,
            high_commit_qc: backup.high_commit_qc,
            high_timeout_qc: backup.high_timeout_qc,
            block_proposal_cache,
            commit_views_cache: BTreeMap::new(),
            commit_qcs_cache: BTreeMap::new(),
            timeout_views_cache: BTreeMap::new(),
            timeout_qcs_cache: BTreeMap::new(),
            view_start: ctx.now(),
        };

        Ok(this)
    }

    /// Runs a loop to process incoming messages (may be `None` if the channel times out while waiting for a message).
    /// This is the main entry point for the state machine,
    /// potentially triggering state modifications and message sending to the executor.
    pub(crate) async fn run(mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        tracing::trace!("Starting ChonkyBFT replica.");
        self.view_start = ctx.now();

        // If this is the first view, we immediately timeout. This will force the replicas
        // to synchronize right at the beginning and will provide a justification for the
        // next view. This is necessary because the first view is not justified by any
        // previous view.
        if self.view_number == validator::ViewNumber(0) {
            tracing::trace!("ChonkyBFT replica - Starting view 0, immediately timing out.");
            self.start_timeout(ctx).await?;
        }

        // Main loop.
        loop {
            let recv = self
                .inbound_channel
                .recv(&ctx.with_deadline(self.view_timeout))
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

            // Process the message.
            let now = ctx.now();

            // Unwrap the v2 message from the others.
            #[allow(irrefutable_let_patterns)]
            let validator::ConsensusMsg::V2(msg) = &req.msg.msg
            else {
                tracing::debug!(
                    "ChonkyBFT replica - Received a message from a different protocol version.",
                );
                continue;
            };

            let label = match msg {
                validator::v2::ChonkyMsg::LeaderProposal(_) => {
                    let res = match self
                        .on_proposal(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("on_proposal()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                // If the error is internal, we stop here.
                                proposal::Error::Internal(err) => {
                                    match &err {
                                        ctx::Error::Canceled(_) => {
                                            tracing::trace!(
                                                "ChonkyBFT replica - on_proposal: canceled"
                                            );
                                        }
                                        ctx::Error::Internal(error) => {
                                            tracing::error!(
                                                "ChonkyBFT replica - on_proposal: internal error: \
                                                 {error:#}"
                                            );
                                        }
                                    }
                                    return Err(err);
                                }
                                // If the error is due to an old message, we log it at a lower level.
                                proposal::Error::Old { .. } => {
                                    tracing::debug!("ChonkyBFT replica - on_proposal: {err:#}");
                                }
                                _ => {
                                    tracing::debug!("ChonkyBFT replica - on_proposal: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::LeaderProposal.with_result(&res)
                }
                validator::v2::ChonkyMsg::ReplicaCommit(_) => {
                    let res = match self
                        .on_commit(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("on_commit()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                // If the error is internal, we stop here.
                                commit::Error::Internal(err) => {
                                    match &err {
                                        ctx::Error::Canceled(_) => {
                                            tracing::trace!(
                                                "ChonkyBFT replica - on_commit: canceled"
                                            );
                                        }
                                        ctx::Error::Internal(error) => {
                                            tracing::error!(
                                                "ChonkyBFT replica - on_commit: internal error: \
                                                 {error:#}"
                                            );
                                        }
                                    }
                                    return Err(err);
                                }
                                // If the error is due to an old message, we log it at a lower level.
                                commit::Error::Old { .. } => {
                                    tracing::debug!("ChonkyBFT replica - on_commit: {err:#}");
                                }
                                _ => {
                                    tracing::debug!("ChonkyBFT replica - on_commit: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::ReplicaCommit.with_result(&res)
                }
                validator::v2::ChonkyMsg::ReplicaTimeout(_) => {
                    let res = match self
                        .on_timeout(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("on_timeout()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                // If the error is internal, we stop here.
                                timeout::Error::Internal(err) => {
                                    match &err {
                                        ctx::Error::Canceled(_) => {
                                            tracing::trace!(
                                                "ChonkyBFT replica - on_timeout: canceled"
                                            );
                                        }
                                        ctx::Error::Internal(error) => {
                                            tracing::error!(
                                                "ChonkyBFT replica - on_timeout: internal error: \
                                                 {error:#}"
                                            );
                                        }
                                    }
                                    return Err(err);
                                }
                                // If the error is due to an old message, we log it at a lower level.
                                timeout::Error::Old { .. } => {
                                    tracing::debug!("ChonkyBFT replica - on_timeout: {err:#}");
                                }
                                _ => {
                                    tracing::debug!("ChonkyBFT replica - on_timeout: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::ReplicaTimeout.with_result(&res)
                }
                validator::v2::ChonkyMsg::ReplicaNewView(_) => {
                    let res = match self
                        .on_new_view(ctx, req.msg.cast().unwrap())
                        .await
                        .wrap("on_new_view()")
                    {
                        Ok(()) => Ok(()),
                        Err(err) => {
                            match err {
                                // If the error is internal, we stop here.
                                new_view::Error::Internal(err) => {
                                    match &err {
                                        ctx::Error::Canceled(_) => {
                                            tracing::trace!(
                                                "ChonkyBFT replica - on_new_view: canceled"
                                            );
                                        }
                                        ctx::Error::Internal(error) => {
                                            tracing::error!(
                                                "ChonkyBFT replica - on_new_view: internal error: \
                                                 {error:#}"
                                            );
                                        }
                                    }
                                    return Err(err);
                                }
                                // If the error is due to an old message, we log it at a lower level.
                                new_view::Error::Old { .. } => {
                                    tracing::debug!("ChonkyBFT replica - on_new_view: {err:#}");
                                }
                                _ => {
                                    tracing::debug!("ChonkyBFT replica - on_new_view: {err:#}");
                                }
                            }
                            Err(())
                        }
                    };
                    metrics::ConsensusMsgLabel::ReplicaNewView.with_result(&res)
                }
            };
            metrics::METRICS.message_processing_latency[&label].observe_latency(ctx.now() - now);

            // Notify network component that the message has been processed.
            // Ignore sending error.
            let _ = req.ack.send(());
        }
    }

    /// Processes a (already verified) CommitQC. It bumps the local high_commit_qc and if
    /// we have the proposal corresponding to this qc, we save the corresponding block to DB.
    pub(crate) async fn process_commit_qc(
        &mut self,
        ctx: &ctx::Ctx,
        qc: &validator::v2::CommitQC,
    ) -> ctx::Result<()> {
        if self
            .high_commit_qc
            .as_ref()
            .is_none_or(|cur| cur.view().number < qc.view().number)
        {
            tracing::trace!(
                "ChonkyBFT replica - Processing newer CommitQC: current view {}, QC view {}",
                self.view_number.0,
                qc.view().number.0,
            );

            self.high_commit_qc = Some(qc.clone());
            self.save_block(ctx, qc).await.wrap("save_block()")?;
        }

        Ok(())
    }

    /// Processes a (already verified) TimeoutQC. It bumps the local high_timeout_qc.
    pub(crate) async fn process_timeout_qc(
        &mut self,
        ctx: &ctx::Ctx,
        qc: &validator::v2::TimeoutQC,
    ) -> ctx::Result<()> {
        if let Some(high_qc) = qc.high_qc() {
            self.process_commit_qc(ctx, high_qc)
                .await
                .wrap("process_commit_qc()")?;
        }

        if self
            .high_timeout_qc
            .as_ref()
            .is_none_or(|old| old.view.number < qc.view.number)
        {
            tracing::trace!(
                "ChonkyBFT replica - Processing newer TimeoutQC: current view {}, QC view {}",
                self.view_number.0,
                qc.view.number.0,
            );

            self.high_timeout_qc = Some(qc.clone());
        }

        Ok(())
    }
}
