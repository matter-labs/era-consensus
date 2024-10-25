use std::cmp::max;

use super::StateMachine;
use crate::metrics;
use zksync_concurrency::{ctx, error::Wrap, time};
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator;

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
    InvalidMessage(#[source] validator::ReplicaNewViewVerifyError),
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
        signed_message: validator::Signed<validator::ReplicaNewView>,
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
        if message.view().number < self.view_number {
            return Err(Error::Old {
                current_view: self.view_number,
            });
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message.verify().map_err(Error::InvalidSignature)?;

        message
            .verify(self.config.genesis())
            .map_err(Error::InvalidMessage)?;

        // ----------- All checks finished. Now we process the message. --------------

        // Update the state machine.
        match &message.justification {
            validator::ProposalJustification::Commit(qc) => self
                .process_commit_qc(ctx, qc)
                .await
                .wrap("process_commit_qc()")?,
            validator::ProposalJustification::Timeout(qc) => {
                if let Some(high_qc) = qc.high_qc() {
                    self.process_commit_qc(ctx, high_qc)
                        .await
                        .wrap("process_commit_qc()")?;
                }
                self.high_timeout_qc = max(Some(qc.clone()), self.high_timeout_qc.clone());
            }
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
        // Update the state machine.
        self.view_number = view;
        self.phase = validator::Phase::Prepare;

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
                    validator::ReplicaNewView {
                        justification: self.get_justification(),
                    },
                )),
        };
        self.outbound_pipe.send(output_message.into());

        // Log the event.
        tracing::info!("Starting view {}", self.view_number);
        metrics::METRICS.replica_view_number.set(self.view_number.0);

        // Reset the timeout.
        self.timeout_deadline = time::Deadline::Finite(ctx.now() + Self::TIMEOUT_DURATION);

        Ok(())
    }
}
