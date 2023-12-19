use super::StateMachine;
use tracing::instrument;
use zksync_concurrency::{ctx, error::Wrap};
use zksync_consensus_roles::validator::{self, ProtocolVersion};

/// Errors that can occur when processing a "replica prepare" message.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// Incompatible protocol version.
    #[error("incompatible protocol version (message version: {message_version:?}, local version: {local_version:?}")]
    IncompatibleProtocolVersion {
        /// Message version.
        message_version: ProtocolVersion,
        /// Local version.
        local_version: ProtocolVersion,
    },
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
    /// The node is not a leader for this message's view.
    #[error("we are not a leader for this message's view")]
    NotLeaderInView,
    /// Duplicate message from a replica.
    #[error("duplicate message from a replica (existing message: {existing_message:?}")]
    Exists {
        /// Existing message from the same replica.
        existing_message: validator::ReplicaPrepare,
    },
    /// High QC of a future view.
    #[error(
        "high QC of a future view (high QC view: {high_qc_view:?}, current view: {current_view:?}"
    )]
    HighQCOfFutureView {
        /// Received high QC view.
        high_qc_view: validator::ViewNumber,
        /// Current view.
        current_view: validator::ViewNumber,
    },
    /// Invalid message signature.
    #[error("invalid signature: {0:#}")]
    InvalidSignature(#[source] validator::Error),
    /// Invalid `HighQC` message.
    #[error("invalid high QC: {0:#}")]
    InvalidHighQC(#[source] anyhow::Error),
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

        // Check protocol version compatibility.
        if !crate::PROTOCOL_VERSION.compatible(&message.protocol_version) {
            return Err(Error::IncompatibleProtocolVersion {
                message_version: message.protocol_version,
                local_version: crate::PROTOCOL_VERSION,
            });
        }

        // Check that the message signer is in the validator set.
        let validator_index =
            self.inner
                .validator_set
                .index(author)
                .ok_or(Error::NonValidatorSigner {
                    signer: author.clone(),
                })?;

        // If the message is from the "past", we discard it.
        if (message.view, validator::Phase::Prepare) < (self.view, self.phase) {
            return Err(Error::Old {
                current_view: self.view,
                current_phase: self.phase,
            });
        }

        // If the message is for a view when we are not a leader, we discard it.
        if self.inner.view_leader(message.view) != self.inner.secret_key.public() {
            return Err(Error::NotLeaderInView);
        }

        // If we already have a message from the same validator and for the same view, we discard it.
        if let Some(existing_message) = self
            .prepare_message_cache
            .get(&message.view)
            .and_then(|x| x.get(author))
        {
            return Err(Error::Exists {
                existing_message: existing_message.msg.clone(),
            });
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message.verify().map_err(Error::InvalidSignature)?;

        // ----------- Checking the contents of the message --------------

        // Verify the high QC.
        message
            .high_qc
            .verify(&self.inner.validator_set, self.inner.threshold())
            .map_err(Error::InvalidHighQC)?;

        // If the high QC is for a future view, we discard the message.
        // This check is not necessary for correctness, but it's useful to
        // guarantee that our proposals don't contain QCs from the future.
        if message.high_qc.message.view >= message.view {
            return Err(Error::HighQCOfFutureView {
                high_qc_view: message.high_qc.message.view,
                current_view: message.view,
            });
        }

        // ----------- All checks finished. Now we process the message. --------------

        // We add the message to the incrementally-constructed QC.
        self.prepare_qcs.entry(message.view).or_default().add(
            &signed_message,
            (validator_index, self.inner.validator_set.len()),
        );

        // We store the message in our cache.
        self.prepare_message_cache
            .entry(message.view)
            .or_default()
            .insert(author.clone(), signed_message);

        // Now we check if we have enough messages to continue.
        let num_messages = self.prepare_message_cache.get(&message.view).unwrap().len();

        if num_messages < self.inner.threshold() {
            return Ok(());
        }

        // Remove replica prepare messages for this view, so that we don't create a new block proposal
        // for this same view if we receive another replica prepare message after this.
        self.prepare_message_cache.remove(&message.view);

        debug_assert_eq!(num_messages, self.inner.threshold());

        // ----------- Update the state machine --------------

        self.view = message.view;
        self.phase = validator::Phase::Commit;
        self.phase_start = ctx.now();

        // Consume the incrementally-constructed QC for this view.
        let justification = self.prepare_qcs.remove(&message.view).unwrap();

        self.prepare_qc.send_replace(Some(justification));
        Ok(())
    }
}
