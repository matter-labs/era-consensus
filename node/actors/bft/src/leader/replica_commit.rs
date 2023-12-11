use super::StateMachine;
use crate::{metrics};
use tracing::instrument;
use zksync_concurrency::{ctx, metrics::LatencyHistogramExt as _};
use zksync_consensus_network::io::{ConsensusInputMessage, Target};
use zksync_consensus_roles::validator::{self, ProtocolVersion};
use std::collections::HashMap;

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
    /// Unexpected proposal.
    #[error("unexpected proposal")]
    UnexpectedProposal,
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
    /// Duplicate message from a replica.
    #[error("duplicate message from a replica (existing message: {existing_message:?}")]
    DuplicateMessage {
        /// Existing message from the same replica.
        existing_message: validator::ReplicaCommit,
    },
    /// Number of received messages is below threshold.
    #[error(
        "number of received messages is below threshold. waiting for more (received: {num_messages:?}, \
         threshold: {threshold:?}"
    )]
    NumReceivedBelowThreshold {
        /// Number of received messages.
        num_messages: usize,
        /// Threshold for message count.
        threshold: usize,
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
        let message = signed_message.msg;
        let author = &signed_message.key;

        // Check protocol version compatibility.
        if !crate::PROTOCOL_VERSION.compatible(&message.protocol_version) {
            return Err(Error::IncompatibleProtocolVersion {
                message_version: message.protocol_version,
                local_version: crate::PROTOCOL_VERSION,
            });
        }

        // Check that the message signer is in the validator set.
        self.inner 
            .validator_set
            .index(author)
            .ok_or(Error::NonValidatorSigner {
                signer: author.clone(),
            })?;

        // If the message is from the "past", we discard it.
        if (message.view, validator::Phase::Commit) < (self.view, self.phase) {
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
            .commit_message_cache
            .get(&message.view)
            .and_then(|x| x.get(author))
        {
            return Err(Error::DuplicateMessage {
                existing_message: existing_message.msg,
            });
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message.verify().map_err(Error::InvalidSignature)?;

        // ----------- Checking the contents of the message --------------

        // We only accept replica commit messages for proposals that we have cached. That's so
        // we don't need to store replica commit messages for different proposals.
        if self.block_proposal_cache != Some(message.proposal) {
            return Err(Error::UnexpectedProposal);
        }

        // ----------- All checks finished. Now we process the message. --------------

        // We store the message in our cache.
        self.commit_message_cache
            .entry(message.view)
            .or_default()
            .insert(author.clone(), signed_message);

        // Now we check if we have enough messages to continue.
        let mut by_proposal: HashMap<_, Vec<_>> = HashMap::new();
        for msg in self.commit_message_cache.get(&message.view).unwrap().iter() {
            by_proposal.entry(msg.proposal.clone()).or_default().push(msg);
        }
        let Some((proposal,replica_messages)) = by_proposal.into_iter().find(|(k,v)|v.len()>=self.inner.threshold()) else {
            return Ok(());
        };
        debug_assert!(replica_messages.len() == self.inner.threshold());

        // ----------- Update the state machine --------------

        let now = ctx.now();
        metrics::METRICS
            .leader_commit_phase_latency
            .observe_latency(now - self.phase_start);
        self.view = message.view.next();
        self.phase = validator::Phase::Prepare;
        self.phase_start = now;

        // ----------- Prepare our message and send it. --------------

        // Create the justification for our message.
        let justification = validator::CommitQC::from(
            &replica_messages.iter().cloned().collect::<Vec<_>>(),
            &self.inner.validator_set
        ).expect("Couldn't create justification from valid replica messages!");

        // Broadcast the leader commit message to all replicas (ourselves included).
        let output_message = ConsensusInputMessage {
            message: self.inner
                .secret_key
                .sign_msg(validator::ConsensusMsg::LeaderCommit(
                    validator::LeaderCommit {
                        protocol_version: crate::PROTOCOL_VERSION,
                        justification,
                    },
                )),
            recipient: Target::Broadcast,
        };
        self.inner.pipe.send(output_message.into());

        // Clean the caches.
        self.prepare_message_cache.retain(|k, _| k >= &self.view);
        self.commit_message_cache.retain(|k, _| k >= &self.view);

        Ok(())
    }
}
