use zksync_concurrency::{ctx, error::Wrap, metrics::LatencyHistogramExt as _, time};
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator;

use super::StateMachine;
use crate::metrics;

/// Errors that can occur when processing a ReplicaNewView message.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// Message signer isn't part of the validator set.
    #[error("message signer isn't part of the validator set (signer: {signer:?})")]
    NonValidatorSigner {
        /// Signer of the message.
        signer: Box<validator::PublicKey>,
    },
    /// Past view or phase.
    #[error("past view (current view: {current_view:?})")]
    Old {
        /// Current view.
        current_view: validator::ViewNumber,
    },
    /// Invalid message signature.
    #[error("invalid signature: {0:#}")]
    InvalidSignature(#[source] anyhow::Error),
    /// Invalid message.
    #[error("invalid message: {0:#}")]
    InvalidMessage(#[source] validator::v1::ReplicaNewViewVerifyError),
    /// Internal error. Unlike other error types, this one isn't supposed to be easily recoverable.
    #[error(transparent)]
    Internal(#[from] ctx::Error),
}

impl Wrap for Error {
    fn with_wrap<C: std::fmt::Display + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        f: F,
    ) -> Self {
        match self {
            Error::Internal(err) => Error::Internal(err.with_wrap(f)),
            err => err,
        }
    }
}

impl StateMachine {
    /// Processes a ReplicaNewView message.
    pub(crate) async fn on_new_view(
        &mut self,
        ctx: &ctx::Ctx,
        signed_message: validator::Signed<validator::v1::ReplicaNewView>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;

        // If the replica is already in this view (and the message is NOT from the leader), then ignore it.
        // See `start_new_view()` for explanation why we need to process the message from the
        // leader.
        if message.view().number < self.view_number
            || (message.view().number == self.view_number
                && author
                    != &self
                        .config
                        .genesis()
                        .validators_schedule
                        .as_ref()
                        .unwrap()
                        .view_leader(self.view_number))
        {
            return Err(Error::Old {
                current_view: self.view_number,
            });
        }

        // Check that the message signer is in the validator committee.
        if !self.config.genesis().validators.contains(author) {
            return Err(Error::NonValidatorSigner {
                signer: author.clone().into(),
            });
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message.verify().map_err(Error::InvalidSignature)?;

        message
            .verify(self.config.genesis())
            .map_err(Error::InvalidMessage)?;

        // ----------- All checks finished. Now we process the message. --------------

        tracing::debug!(
            bft_message = format!("{:#?}", message),
            "ChonkyBFT replica - Received a new view message from {author:#?}.",
        );

        // Update the state machine.
        match &message.justification {
            validator::v1::ProposalJustification::Commit(qc) => self
                .process_commit_qc(ctx, qc)
                .await
                .wrap("process_commit_qc()")?,
            validator::v1::ProposalJustification::Timeout(qc) => self
                .process_timeout_qc(ctx, qc)
                .await
                .wrap("process_timeout_qc()")?,
        };

        // If the message is for a future view, we need to start a new view.
        if message.view().number > self.view_number {
            self.start_new_view(ctx, message.view().number).await?;
        }

        Ok(())
    }

    /// This blocking method is used whenever we start a new view.
    pub(crate) async fn start_new_view(
        &mut self,
        ctx: &ctx::Ctx,
        view: validator::ViewNumber,
    ) -> ctx::Result<()> {
        tracing::info!("ChonkyBFT replica - Starting view number {}.", view);

        // Update the state machine.
        self.view_number = view;
        metrics::METRICS.replica_view_number.set(self.view_number.0);
        self.phase = validator::v1::Phase::Prepare;

        // It is important that the proposal and new_view messages from the leader
        // will contain the same justification.
        // Proposal cannot be produced before previous block is processed,
        // therefore leader needs to make sure that the high_commit_qc is delivered
        // to all replicas, so that the finalized block is distributed over the network.
        // In particular it is not guaranteed that the leader has the finalized block when
        // sending the NewView, so it might need to wait for the finalized block.
        //
        // Note that for this process to work e2e, the replicas should NOT ignore the NewView from
        // the leader, even if they already advanced to the given view.
        // Note that the order of NewView and proposal messages doesn't matter, because
        // proposal is a superset of NewView message.
        let justification = self.get_justification();
        self.proposer_sender
            .send(Some(justification.clone()))
            .expect("justification_watch.send() failed");

        // Clear the block proposal cache.
        if let Some(qc) = self.high_commit_qc.as_ref() {
            self.block_proposal_cache
                .retain(|k, _| k > &qc.header().number);
        }

        // Backup our state.
        self.backup_state(ctx).await.wrap("backup_state()")?;

        // Broadcast our new view message.
        let output_message = ConsensusInputMessage {
            message: self
                .config
                .secret_key
                .sign_msg(validator::ConsensusMsg::ReplicaNewView(
                    validator::v1::ReplicaNewView {
                        justification: justification.clone(),
                    },
                )),
        };
        tracing::debug!(
            bft_message = format!("{:#?}", output_message.message),
            "ChonkyBFT replica - Broadcasting new view message at start of view.",
        );
        self.outbound_channel.send(output_message);

        // Update the metrics.
        let now = ctx.now();
        metrics::METRICS
            .view_latency
            .observe_latency(now - self.view_start);
        self.view_start = now;

        // Reset the timeout.
        self.view_timeout = time::Deadline::Finite(ctx.now() + self.config.view_timeout);

        Ok(())
    }

    /// Makes a justification (for a ReplicaNewView or a LeaderProposal) based on the current state.
    pub(crate) fn get_justification(&self) -> validator::v1::ProposalJustification {
        // We need some QC in order to be able to create a justification.
        // In fact, it should be impossible to get here without a QC. Because
        // we only get here after starting a new view, which requires a QC.
        assert!(self.high_commit_qc.is_some() || self.high_timeout_qc.is_some());

        // We use the highest QC as the justification. If both have the same view, we use the CommitQC.
        if self.high_commit_qc.as_ref().map(|x| x.view())
            >= self.high_timeout_qc.as_ref().map(|x| &x.view)
        {
            validator::v1::ProposalJustification::Commit(self.high_commit_qc.clone().unwrap())
        } else {
            validator::v1::ProposalJustification::Timeout(self.high_timeout_qc.clone().unwrap())
        }
    }
}
