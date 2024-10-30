use super::StateMachine;
use crate::metrics;
use std::{cmp::max, collections::HashSet};
use zksync_concurrency::{ctx, error::Wrap, metrics::LatencyHistogramExt as _, time};
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator;

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
    InvalidMessage(#[source] validator::ReplicaTimeoutVerifyError),
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
        signed_message: validator::Signed<validator::ReplicaTimeout>,
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
            .verify(self.config.genesis())
            .map_err(Error::InvalidMessage)?;

        // ----------- All checks finished. Now we process the message. --------------

        // We add the message to the incrementally-constructed QC.
        let timeout_qc = self
            .timeout_qcs_cache
            .entry(message.view.number)
            .or_insert_with(|| validator::TimeoutQC::new(message.view));

        // Should always succeed as all checks have been already performed
        timeout_qc
            .add(&signed_message, self.config.genesis())
            .expect("could not add message to TimeoutQC");

        // Calculate the TimeoutQC signers weight.
        let weight = timeout_qc.weight(&self.config.genesis().validators);

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
        if weight < self.config.genesis().validators.quorum_threshold() {
            return Ok(());
        };

        // ----------- We have a QC. Now we process it. --------------

        // Consume the created timeout QC for this view.
        let timeout_qc = self.timeout_qcs_cache.remove(&message.view.number).unwrap();

        // We update our state with the new timeout QC.
        if let Some(commit_qc) = timeout_qc.high_qc() {
            self.process_commit_qc(ctx, commit_qc)
                .await
                .wrap("process_commit_qc()")?;
        }
        self.high_timeout_qc = max(Some(timeout_qc.clone()), self.high_timeout_qc.clone());

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

    /// This blocking method is used whenever we timeout in a view.
    pub(crate) async fn start_timeout(&mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        // Update the state machine.
        self.phase = validator::Phase::Timeout;

        // Backup our state.
        self.backup_state(ctx).await.wrap("backup_state()")?;

        // Broadcast our timeout message.
        let output_message = ConsensusInputMessage {
            message: self
                .config
                .secret_key
                .sign_msg(validator::ConsensusMsg::ReplicaTimeout(
                    validator::ReplicaTimeout {
                        view: validator::View {
                            genesis: self.config.genesis().hash(),
                            number: self.view_number,
                        },
                        high_vote: self.high_vote.clone(),
                        high_qc: self.high_commit_qc.clone(),
                    },
                )),
        };

        self.outbound_pipe.send(output_message.into());

        // Log the event.
        tracing::info!("Timed out at view {}", self.view_number);
        metrics::METRICS.replica_view_number.set(self.view_number.0);

        // Reset the timeout. This makes us keep sending timeout messages until the consensus progresses.
        // However, this isn't strictly necessary since the network retries messages until they are delivered.
        // This is just an extra safety measure.
        self.timeout_deadline = time::Deadline::Finite(ctx.now() + Self::TIMEOUT_DURATION);

        Ok(())
    }
}
