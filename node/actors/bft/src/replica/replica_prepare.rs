//! Handler of a ReplicaPrepare message.
use super::StateMachine;
use tracing::instrument;
use zksync_concurrency::{ctx, error::Wrap};
use zksync_consensus_roles::validator::{self};

/// Errors that can occur when processing a "replica prepare" message.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// Message signer isn't part of the validator set.
    #[error("Message signer isn't part of the validator set (signer: {signer:?})")]
    NonValidatorSigner {
        /// Signer of the message.
        signer: validator::PublicKey,
    },
    /// Past view or phase.
    #[error("past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    Old {
        /// Current view.
        current_view: validator::ViewNumber,
        /// Current phase.
        current_phase: validator::Phase,
    },
    /// Invalid message signature.
    #[error("invalid signature: {0:#}")]
    InvalidSignature(#[source] anyhow::Error),
    /// Invalid message.
    #[error(transparent)]
    InvalidMessage(validator::ReplicaPrepareVerifyError),
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
    #[instrument(level = "trace", skip(self), ret)]
    pub(crate) async fn process_replica_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        signed_message: validator::Signed<validator::ReplicaPrepare>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = signed_message.msg.clone();
        let author = &signed_message.key;

        // Check that the message signer is in the validator set.
        if !self.config.genesis().validators.contains(author) {
            return Err(Error::NonValidatorSigner {
                signer: author.clone(),
            });
        }

        // We only accept this type of message from the future.
        if message.view.number <= self.view {
            return Err(Error::Old {
                current_view: self.view,
                current_phase: self.phase,
            });
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message.verify().map_err(Error::InvalidSignature)?;

        // Verify the message.
        message
            .verify(self.config.genesis())
            .map_err(Error::InvalidMessage)?;

        // ----------- All checks finished. Now we process the message. --------------

        // Update our high QC, if necessary.
        let qc_view = message.high_qc.clone().map(|x| x.view().number);

        if qc_view > self.high_qc.clone().map(|x| x.view().number) {
            self.high_qc.clone_from(&message.high_qc)
        }

        // Skip to a new view, if necessary.
        if qc_view > Some(self.view) {
            self.view = qc_view.unwrap();
            self.start_new_view(ctx).await.wrap("start_new_view()")?;
        }

        Ok(())
    }
}
