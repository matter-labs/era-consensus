//! Handler of a ReplicaCommit message.

use std::collections::HashSet;

use super::StateMachine;
use crate::metrics;
use tracing::instrument;
use zksync_concurrency::{ctx, metrics::LatencyHistogramExt as _};
use zksync_consensus_network::io::{ConsensusInputMessage, Target};
use zksync_consensus_roles::validator;

/// Errors that can occur when processing a "replica commit" message.
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
    /// The processing node is not a lead for this message's view.
    #[error("we are not a leader for this message's view")]
    NotLeaderInView,
    /// Invalid message.
    #[error("invalid message: {0:#}")]
    InvalidMessage(#[source] validator::ReplicaCommitVerifyError),
    /// Invalid message signature.
    #[error("invalid signature: {0:#}")]
    InvalidSignature(#[source] anyhow::Error),
}

impl StateMachine {
    #[instrument(level = "trace", skip(self), ret)]
    pub(crate) fn process_replica_commit(
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
                signer: author.clone(),
            });
        }

        // If the message is from the "past", we discard it.
        // That is, it's from a previous view or phase, or if we already received a message
        // from the same validator and for the same view.
        if (message.view.number, validator::Phase::Commit) < (self.view, self.phase)
            || self
                .replica_commit_views
                .get(author)
                .is_some_and(|view_number| *view_number >= message.view.number)
        {
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

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message.verify().map_err(Error::InvalidSignature)?;

        message
            .verify(self.config.genesis())
            .map_err(Error::InvalidMessage)?;

        // ----------- All checks finished. Now we process the message. --------------

        // We add the message to the incrementally-constructed QC.
        let commit_qc = self
            .commit_qcs
            .entry(message.view.number)
            .or_default()
            .entry(message.clone())
            .or_insert_with(|| validator::CommitQC::new(message.clone(), self.config.genesis()));

        // Should always succeed as all checks have been already performed
        commit_qc
            .add(&signed_message, self.config.genesis())
            .expect("Could not add message to CommitQC");

        // Calculate the CommitQC signers weight.
        let weight = self.config.genesis().validators.weight(&commit_qc.signers);

        // Update commit message current view number for author
        self.replica_commit_views
            .insert(author.clone(), message.view.number);

        // Clean up commit_qcs for the case that no replica is at the view
        // of a given CommitQC
        // This prevents commit_qcs map from growing indefinitely in case some
        // malicious replica starts spamming messages for future views
        let active_views: HashSet<_> = self.replica_commit_views.values().collect();
        self.commit_qcs
            .retain(|view_number, _| active_views.contains(view_number));

        // Now we check if we have enough weight to continue.
        if weight < self.config.genesis().validators.threshold() {
            return Ok(());
        };

        // ----------- Update the state machine --------------
        let now = ctx.now();
        metrics::METRICS
            .leader_commit_phase_latency
            .observe_latency(now - self.phase_start);
        self.view = message.view.number.next();
        self.phase = validator::Phase::Prepare;
        self.phase_start = now;

        // ----------- Prepare our message and send it. --------------

        // Consume the incrementally-constructed QC for this view.
        let justification = self
            .commit_qcs
            .remove(&message.view.number)
            .unwrap()
            .remove(message)
            .unwrap();

        // Broadcast the leader commit message to all replicas (ourselves included).
        let output_message = ConsensusInputMessage {
            message: self
                .config
                .secret_key
                .sign_msg(validator::ConsensusMsg::LeaderCommit(
                    validator::LeaderCommit { justification },
                )),
            recipient: Target::Broadcast,
        };
        self.outbound_pipe.send(output_message.into());

        Ok(())
    }
}
