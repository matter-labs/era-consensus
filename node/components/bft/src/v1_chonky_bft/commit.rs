use std::collections::HashSet;

use zksync_concurrency::{ctx, error::Wrap, metrics::LatencyHistogramExt as _};
use zksync_consensus_roles::validator;

use super::StateMachine;
use crate::metrics;

/// Errors that can occur when processing a ReplicaCommit message.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// Message signer isn't part of the validator set.
    #[error("message signer isn't part of the validator set (signer: {signer:?})")]
    NonValidatorSigner {
        /// Signer of the message.
        signer: Box<validator::PublicKey>,
    },
    /// Past view.
    #[error("past view (current view: {current_view:?})")]
    Old {
        /// Current view.
        current_view: validator::ViewNumber,
    },
    /// Duplicate signer. We already have a commit message from the same validator
    /// for the same or past view.
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
    InvalidMessage(#[source] validator::v1::ReplicaCommitVerifyError),
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
        signed_message: validator::Signed<validator::v1::ReplicaCommit>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;

        // Check that the message signer is in the validator committee.
        if !self
            .config
            .genesis()
            .validators_schedule
            .as_ref()
            .unwrap()
            .contains(author)
        {
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
        tracing::debug!(
            bft_message = format!("{:#?}", message),
            "ChonkyBFT replica - Received a commit message from {author:#?}."
        );
        let commit_qc = self
            .commit_qcs_cache
            .entry(message.view.number)
            .or_default()
            .entry(message.clone())
            .or_insert_with(|| {
                validator::v1::CommitQC::new(message.clone(), self.config.genesis())
            });

        // Should always succeed as all checks have been already performed
        commit_qc
            .add(&signed_message, self.config.genesis())
            .expect("could not add message to CommitQC");

        // Calculate the CommitQC signers weight.
        let weight = commit_qc
            .signers
            .weight(&validator::v1::get_committee_from_schedule(
                self.config.genesis().validators_schedule.as_ref().unwrap(),
            ));

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
        if weight
            < self
                .config
                .genesis()
                .validators_schedule
                .as_ref()
                .unwrap()
                .quorum_threshold()
        {
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

        tracing::info!("ChonkyBFT replica - We have a commit QC with weight {} at view {} for block number {} with hash {:#?}.",
            weight,
            commit_qc.view().number.0,
            commit_qc.message.proposal.number.0,
            commit_qc.message.proposal.payload,
        );

        // We update our state with the new commit QC.
        self.process_commit_qc(ctx, &commit_qc)
            .await
            .wrap("process_commit_qc()")?;

        // Metrics. We observe the latency of committing to a block measured
        // from the start of this view.
        metrics::METRICS
            .commit_latency
            .observe_latency(ctx.now() - self.view_start);

        // Start a new view.
        self.start_new_view(ctx, message.view.number.next()).await?;

        Ok(())
    }
}
