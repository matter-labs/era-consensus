//! Handler of a ReplicaCommit message.
use super::StateMachine;
use crate::metrics;
use std::collections::HashMap;
use tracing::instrument;
use zksync_concurrency::{ctx, metrics::LatencyHistogramExt as _};
use zksync_consensus_network::io::{ConsensusInputMessage, Target};
use zksync_consensus_roles::validator::{self, CommitQC, ProtocolVersion};

/// Errors that can occur when processing a "replica commit" message.
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
    /// The processing node is not a lead for this message's view.
    #[error("we are not a leader for this message's view")]
    NotLeaderInView,
    /// Invalid message.
    #[error("invalid message: {0:#}")]
    InvalidMessage(anyhow::Error),
    /// Duplicate message from a replica.
    #[error("duplicate message from a replica (existing message: {existing_message:?}")]
    DuplicateMessage {
        /// Existing message from the same replica.
        existing_message: validator::ReplicaCommit,
    },
    /// Invalid message signature.
    #[error("invalid signature: {0:#}")]
    InvalidSignature(#[source] validator::Error),
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

        // Check protocol version compatibility.
        if !crate::PROTOCOL_VERSION.compatible(&message.view.protocol_version) {
            return Err(Error::IncompatibleProtocolVersion {
                message_version: message.view.protocol_version,
                local_version: crate::PROTOCOL_VERSION,
            });
        }

        // Check that the message signer is in the validator set.
        if !self.config.genesis().validators.contains(author) {
            return Err(Error::NonValidatorSigner {
                signer: author.clone(),
            });
        }

        // If the message is from the "past", we discard it.
        if (message.view.number, validator::Phase::Commit) < (self.view, self.phase) {
            return Err(Error::Old {
                current_view: self.view,
                current_phase: self.phase,
            });
        }

        // If the message is for a view when we are not a leader, we discard it.
        if self
            .config
            .genesis()
            .validators
            .view_leader(message.view.number)
            != self.config.secret_key.public()
        {
            return Err(Error::NotLeaderInView);
        }

        // If we already have a message from the same validator and for the same view, we discard it.
        if let Some(existing_message) = self
            .commit_message_cache
            .get(&message.view.number)
            .and_then(|x| x.get(author))
        {
            return Err(Error::DuplicateMessage {
                existing_message: existing_message.msg.clone(),
            });
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message.verify().map_err(Error::InvalidSignature)?;

        message
            .verify(self.config.genesis(), /*allow_past_forks=*/ false)
            .map_err(Error::InvalidMessage)?;

        // ----------- All checks finished. Now we process the message. --------------

        // TODO: we have a bug here since we don't check whether replicas commit
        // to the same proposal.

        // We add the message to the incrementally-constructed QC.
        self.commit_qcs
            .entry(message.view.number)
            .or_insert_with(|| CommitQC::new(message.clone(), self.config.genesis()))
            .add(&signed_message, self.config.genesis());

        // We store the message in our cache.
        let cache_entry = self
            .commit_message_cache
            .entry(message.view.number)
            .or_default();
        cache_entry.insert(author.clone(), signed_message.clone());

        // Now we check if we have enough messages to continue.
        let mut by_proposal: HashMap<_, Vec<_>> = HashMap::new();
        for msg in cache_entry.values() {
            by_proposal.entry(msg.msg.proposal).or_default().push(msg);
        }
        let threshold = self.config.genesis().validators.threshold();
        let Some((_, replica_messages)) =
            by_proposal.into_iter().find(|(_, v)| v.len() >= threshold)
        else {
            return Ok(());
        };
        debug_assert_eq!(replica_messages.len(), threshold);

        // ----------- Update the state machine --------------

        let now = ctx.now();
        metrics::METRICS
            .leader_commit_phase_latency
            .observe_latency(now - self.phase_start);
        self.view = message.view.number.next();
        self.phase = validator::Phase::Prepare;
        self.phase_start = now;

        // ----------- Prepare our message and send it. --------------

        // Remove replica commit messages for this view, so that we don't create a new leader commit
        // for this same view if we receive another replica commit message after this.
        self.commit_message_cache.remove(&message.view.number);

        // Consume the incrementally-constructed QC for this view.
        let justification = self.commit_qcs.remove(&message.view.number).unwrap();

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

        // Clean the caches.
        self.prepare_message_cache.retain(|k, _| k >= &self.view);
        self.commit_message_cache.retain(|k, _| k >= &self.view);

        Ok(())
    }
}
