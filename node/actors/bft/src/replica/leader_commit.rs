use super::StateMachine;
use tracing::instrument;
use zksync_concurrency::{ctx, error::Wrap};
use zksync_consensus_roles::validator::{self, ProtocolVersion};

/// Errors that can occur when processing a "leader commit" message.
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
    /// Invalid leader.
    #[error("bad leader: got {got:?}, want {want:?}")]
    BadLeader {
        /// Received leader.
        got: validator::PublicKey,
        /// Correct leader.
        want: validator::PublicKey,
    },
    /// Past view of phase.
    #[error("past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    Old {
        /// Current view.
        current_view: validator::ViewNumber,
        /// Current phase.
        current_phase: validator::Phase,
    },
    /// Invalid message signature.
    #[error("invalid signature: {0:#}")]
    InvalidSignature(validator::Error),
    /// Invalid message.
    #[error("invalid message: {0:#}")]
    InvalidMessage(validator::CommitQCVerifyError),
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
    /// Processes a leader commit message. We can approve this leader message even if we
    /// don't have the block proposal stored. It is enough to see the justification.
    #[instrument(level = "trace", err)]
    pub(crate) async fn process_leader_commit(
        &mut self,
        ctx: &ctx::Ctx,
        signed_message: validator::Signed<validator::LeaderCommit>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;

        // Check protocol version compatibility.
        if !crate::PROTOCOL_VERSION.compatible(&message.view().protocol_version) {
            return Err(Error::IncompatibleProtocolVersion {
                message_version: message.view().protocol_version,
                local_version: crate::PROTOCOL_VERSION,
            });
        }

        // Check that it comes from the correct leader.
        let leader = self
            .config
            .genesis()
            .validators
            .view_leader(message.view().number);
        if author != &leader {
            return Err(Error::BadLeader {
                want: leader,
                got: author.clone(),
            });
        }

        // If the message is from the "past", we discard it.
        if (message.view().number, validator::Phase::Commit) < (self.view, self.phase) {
            return Err(Error::Old {
                current_view: self.view,
                current_phase: self.phase,
            });
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message.verify().map_err(Error::InvalidSignature)?;
        message
            .verify(&self.config.genesis())
            .map_err(Error::InvalidMessage)?;

        // ----------- All checks finished. Now we process the message. --------------

        // Try to create a finalized block with this CommitQC and our block proposal cache.
        self.save_block(ctx, &message.justification)
            .await
            .wrap("save_block()")?;

        // Start a new view. But first we skip to the view of this message.
        self.view = message.view().number;
        self.start_new_view(ctx).await.wrap("start_new_view()")?;

        Ok(())
    }
}
