use std::collections::HashSet;

use zksync_concurrency::{ctx, error::Wrap, time};
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator;

use super::StateMachine;

/// Errors that can occur when processing a ReplicaTimeout message.
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
    /// Duplicate signer. We already have a timeout message from the same validator
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
    InvalidMessage(#[source] validator::v2::ReplicaTimeoutVerifyError),
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
    /// Processes a ReplicaTimeout message.
    pub(crate) async fn on_timeout(
        &mut self,
        ctx: &ctx::Ctx,
        signed_message: validator::Signed<validator::v2::ReplicaTimeout>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;

        // Check that the message signer is in the validator committee.
        if !self.config.validators.contains(author) {
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
        if let Some(&view) = self.timeout_views_cache.get(author) {
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
            .verify(
                self.config.genesis_hash(),
                self.config.epoch,
                &self.config.validators,
            )
            .map_err(Error::InvalidMessage)?;

        // ----------- All checks finished. Now we process the message. --------------

        // We add the message to the incrementally-constructed QC.
        tracing::debug!(
            bft_message = format!("{:#?}", message),
            "ChonkyBFT replica - Received a timeout message from {author:#?}.",
        );
        let timeout_qc = self
            .timeout_qcs_cache
            .entry(message.view.number)
            .or_insert_with(|| validator::v2::TimeoutQC::new(message.view));

        // Should always succeed as all checks have been already performed
        timeout_qc
            .add(
                &signed_message,
                self.config.genesis_hash(),
                self.config.epoch,
                &self.config.validators,
            )
            .expect("could not add message to TimeoutQC");

        // Calculate the TimeoutQC signers weight.
        let weight = timeout_qc.weight(&self.config.validators);

        // Update view number of last timeout message for author
        self.timeout_views_cache
            .insert(author.clone(), message.view.number);

        // Clean up timeout_qcs for the case that no replica is at the view
        // of a given TimeoutQC
        // This prevents timeout_qcs map from growing indefinitely in case some
        // malicious replica starts spamming messages for future views
        let active_views: HashSet<_> = self.timeout_views_cache.values().collect();
        self.timeout_qcs_cache
            .retain(|view_number, _| active_views.contains(view_number));

        // Now we check if we have enough weight to continue. If not, we wait for more messages.
        if weight < self.config.validators.quorum_threshold() {
            return Ok(());
        };
        // ----------- We have a QC. Now we process it. --------------

        // Consume the created timeout QC for this view.
        let timeout_qc = self.timeout_qcs_cache.remove(&message.view.number).unwrap();

        tracing::info!(
            "ChonkyBFT replica - We have a timeout QC with weight {} for view number {}.",
            weight,
            timeout_qc.view.number.0
        );

        // We update our state with the new timeout QC.
        self.process_timeout_qc(ctx, &timeout_qc)
            .await
            .wrap("process_timeout_qc()")?;

        // Start a new view.
        self.start_new_view(ctx, message.view.number.next()).await?;

        Ok(())
    }

    /// This blocking method is used whenever we timeout in a view.
    pub(crate) async fn start_timeout(&mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        tracing::info!(
            "ChonkyBFT replica - Timed out at view {}.",
            self.view_number
        );

        // Update the state machine. We only reset the view timer so that replicas
        // keep trying to resend timeout messages. This is crucial as we assume that messages
        // are eventually delivered, if timeout messages are dropped and never retried the
        // consensus can stall.
        self.phase = validator::v2::Phase::Timeout;
        self.view_timeout = time::Deadline::Finite(ctx.now() + self.config.view_timeout);

        // Backup our state.
        self.backup_state(ctx).await.wrap("backup_state()")?;

        // Broadcast our new view message. This synchronizes the replicas.
        // We don't broadcast a new view message for view 0 since we don't have
        // a justification for it.
        if self.view_number != validator::ViewNumber(0) {
            let output_message = ConsensusInputMessage {
                message: self.config.secret_key.sign_msg(validator::ConsensusMsg::V2(
                    validator::v2::ChonkyMsg::ReplicaNewView(validator::v2::ReplicaNewView {
                        justification: self.get_justification(),
                    }),
                )),
            };
            tracing::debug!(
                bft_message = format!("{:#?}", output_message.message),
                "ChonkyBFT replica - Broadcasting new view message as part of timeout.",
            );
            self.outbound_channel.send(output_message);
        }

        // Broadcast our timeout message.
        let output_message = ConsensusInputMessage {
            message: self.config.secret_key.sign_msg(validator::ConsensusMsg::V2(
                validator::v2::ChonkyMsg::ReplicaTimeout(validator::v2::ReplicaTimeout {
                    view: validator::v2::View {
                        genesis: self.config.genesis_hash(),
                        number: self.view_number,
                        epoch: self.config.epoch,
                    },
                    high_vote: self.high_vote.clone(),
                    high_qc: self.high_commit_qc.clone(),
                }),
            )),
        };
        tracing::debug!(
            bft_message = format!("{:#?}", output_message.message),
            "ChonkyBFT replica - Broadcasting timeout message.",
        );
        self.outbound_channel.send(output_message);

        Ok(())
    }
}
