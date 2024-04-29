//! Handler of a ReplicaPrepare message.
use super::StateMachine;
use tracing::instrument;
use zksync_concurrency::{ctx, error::Wrap};
use zksync_consensus_roles::validator;

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
    /// The node is not a leader for this message's view.
    #[error("we are not a leader for this message's view")]
    NotLeaderInView,
    /// Duplicate message from a replica.
    #[error("duplicate message from a replica (existing message: {existing_message:?}")]
    Exists {
        /// Existing message from the same replica.
        existing_message: validator::ReplicaPrepare,
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
        if !self.config.genesis().committee.contains(author) {
            return Err(Error::NonValidatorSigner {
                signer: author.clone(),
            });
        }

        // If the message is from the "past", we discard it.
        if (message.view.number, validator::Phase::Prepare) < (self.view, self.phase) {
            return Err(Error::Old {
                current_view: self.view,
                current_phase: self.phase,
            });
        }

        // If the message is for a view when we are not a leader, we discard it.
        if self.config.genesis().view_leader(message.view.number) != self.config.secret_key.public()
        {
            return Err(Error::NotLeaderInView);
        }

        // If we already have a message from the same validator and for the same view, we discard it.
        if let Some(existing_message) = self
            .prepare_message_cache
            .get(&message.view.number)
            .and_then(|x| x.get(author))
        {
            return Err(Error::Exists {
                existing_message: existing_message.msg.clone(),
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

        // We add the message to the incrementally-constructed QC.
        let prepare_qc = self
            .prepare_qcs
            .entry(message.view.number)
            .or_insert_with(|| validator::PrepareQC::new(message.view.clone()));
        prepare_qc.add(&signed_message, self.config.genesis());

        // We store the message in our cache.
        self.prepare_message_cache
            .entry(message.view.number)
            .or_default()
            .insert(author.clone(), signed_message.clone());

        // Now we check if we have enough weight to continue.
        if prepare_qc.weight(&self.config.genesis().committee)
            < self.config.genesis().committee.threshold()
        {
            return Ok(());
        }

        // Remove replica prepare messages for this view, so that we don't create a new block proposal
        // for this same view if we receive another replica prepare message after this.
        self.prepare_message_cache.remove(&message.view.number);

        // ----------- Update the state machine --------------

        self.view = message.view.number;
        self.phase = validator::Phase::Commit;
        self.phase_start = ctx.now();

        // Consume the incrementally-constructed QC for this view.
        let justification = self.prepare_qcs.remove(&message.view.number).unwrap();

        self.prepare_qc.send_replace(Some(justification));
        Ok(())
    }
}
