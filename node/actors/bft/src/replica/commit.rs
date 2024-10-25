use super::StateMachine;
use crate::metrics;
use std::collections::HashSet;
use zksync_concurrency::{ctx, error::Wrap, metrics::LatencyHistogramExt as _};
use zksync_consensus_roles::validator;

/// Errors that can occur when processing a ReplicaCommit message.
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
    /// Duplicate signer.
    #[error("duplicate signer (message view: {message_view:?}, signer: {signer:?})")]
    DuplicateSigner {
        /// View number of the message.
        message_view: validator::ViewNumber,
        /// Signer of the message.
        signer: Box<validator::PublicKey>,
    },
    /// Invalid message signature.
    #[error("invalid signature: {0:#}")]
    InvalidSignature(#[source] anyhow::Error),
    /// Invalid message.
    #[error("invalid message: {0:#}")]
    InvalidMessage(#[source] validator::ReplicaCommitVerifyError),
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
    /// Processes a ReplicaCommit message.
    pub(crate) async fn on_commit(
        &mut self,
        ctx: &ctx::Ctx,
        signed_message: validator::Signed<validator::ReplicaCommit>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;

        // Check that the message signer is in the validator committee.
        if !self.config.genesis().validators.contains(author) {
            return Err(Error::NonValidatorSigner {
                signer: author.clone().into(),
            });
        }

        // If the message is from a past view, ignore it.
        if message.view.number < self.view_number {
            return Err(Error::Old {
                current_view: self.view_number,
            });
        }

        // If we already have a message from the same validator for the same or past view, ignore it.
        if let Some(&view) = self.commit_views_cache.get(author) {
            if view >= message.view.number {
                return Err(Error::DuplicateSigner {
                    message_view: message.view.number,
                    signer: author.clone().into(),
                });
            }
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message.verify().map_err(Error::InvalidSignature)?;

        message
            .verify(self.config.genesis())
            .map_err(Error::InvalidMessage)?;

        // ----------- All checks finished. Now we process the message. --------------

        // We add the message to the incrementally-constructed QC.
        let commit_qc = self
            .commit_qcs_cache
            .entry(message.view.number)
            .or_default()
            .entry(message.clone())
            .or_insert_with(|| validator::CommitQC::new(message.clone(), self.config.genesis()));

        // Should always succeed as all checks have been already performed
        commit_qc
            .add(&signed_message, self.config.genesis())
            .expect("could not add message to CommitQC");

        // Calculate the CommitQC signers weight.
        let weight = self.config.genesis().validators.weight(&commit_qc.signers);

        // Update view number of last commit message for author
        self.commit_views_cache
            .insert(author.clone(), message.view.number);

        // Clean up commit_qcs for the case that no replica is at the view
        // of a given CommitQC.
        // This prevents commit_qcs map from growing indefinitely in case some
        // malicious replica starts spamming messages for future views.
        let active_views: HashSet<_> = self.commit_views_cache.values().collect();
        self.commit_qcs_cache
            .retain(|view_number, _| active_views.contains(view_number));

        // Now we check if we have enough weight to continue. If not, we wait for more messages.
        if weight < self.config.genesis().validators.quorum_threshold() {
            return Ok(());
        };

        // ----------- We have a QC. Now we process it. --------------

        // Consume the created commit QC for this view.
        let commit_qc = self
            .commit_qcs_cache
            .remove(&message.view.number)
            .unwrap()
            .remove(message)
            .unwrap();

        // We update our state with the new commit QC.
        self.process_commit_qc(ctx, &commit_qc)
            .await
            .wrap("process_commit_qc()")?;

        // Metrics.
        let now = ctx.now();
        metrics::METRICS
            .leader_commit_phase_latency
            .observe_latency(now - self.phase_start);
        self.phase_start = now;

        // Start a new view.
        self.start_new_view(ctx, message.view.number.next()).await?;

        Ok(())
    }
}
