use super::StateMachine;
use crate::{inner::ConsensusInner, metrics};
use concurrency::ctx;
use network::io::{ConsensusInputMessage, Target};
use rand::Rng;
use roles::validator;
use std::collections::HashMap;
use tracing::instrument;

/// Errors that can occur when processing a "replica prepare" message.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
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
    /// Number of received messages is below a threshold.
    #[error(
        "number of received messages below threshold. waiting for more (received: {num_messages:?}, \
         threshold: {threshold:?}"
    )]
    NumReceivedBelowThreshold {
        /// Number of received messages.
        num_messages: usize,
        /// Threshold for message count.
        threshold: usize,
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
    InvalidSignature(#[source] crypto::bls12_381::Error),
    /// Invalid `HighQC` message.
    #[error("invalid high QC: {0:#}")]
    InvalidHighQC(#[source] anyhow::Error),
}

impl StateMachine {
    #[instrument(level = "trace", ret)]
    pub(crate) fn process_replica_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        signed_message: validator::Signed<validator::ReplicaPrepare>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = signed_message.msg.clone();
        let author = &signed_message.key;

        // If the message is from the "past", we discard it.
        if (message.view, validator::Phase::Prepare) < (self.view, self.phase) {
            return Err(Error::Old {
                current_view: self.view,
                current_phase: self.phase,
            });
        }

        // If the message is for a view when we are not a leader, we discard it.
        if consensus.view_leader(message.view) != consensus.secret_key.public() {
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
            .verify(&consensus.validator_set, consensus.threshold())
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

        // We store the message in our cache.
        self.prepare_message_cache
            .entry(message.view)
            .or_default()
            .insert(author.clone(), signed_message);

        // Now we check if we have enough messages to continue.
        let num_messages = self.prepare_message_cache.get(&message.view).unwrap().len();

        if num_messages < consensus.threshold() {
            return Err(Error::NumReceivedBelowThreshold {
                num_messages,
                threshold: consensus.threshold(),
            });
        }

        // ----------- Creating the block proposal --------------

        // Get all the replica prepare messages for this view. Note that we consume the
        // messages here. That's purposeful, so that we don't create a new block proposal
        // for this same view if we receive another replica prepare message after this.
        let replica_messages: Vec<_> = self
            .prepare_message_cache
            .remove(&message.view)
            .unwrap()
            .into_values()
            .collect();

        debug_assert!(num_messages == consensus.threshold());

        // Get the highest block voted for and check if there's a quorum of votes for it. To have a quorum
        // in this situation, we require 2*f+1 votes, where f is the maximum number of faulty replicas.
        let mut count: HashMap<_, usize> = HashMap::new();

        for vote in replica_messages.iter() {
            *count.entry(vote.msg.high_vote.proposal).or_default() += 1;
        }

        let highest_vote: Option<validator::BlockHeader> = count
            .iter()
            // We only take one value from the iterator because there can only be at most one block with a quorum of 2f+1 votes.
            .find(|(_, v)| **v > 2 * consensus.faulty_replicas())
            .map(|(h, _)| h)
            .cloned();

        // Get the highest CommitQC.
        let highest_qc: &validator::CommitQC = replica_messages
            .iter()
            .map(|s| &s.msg.high_qc)
            .max_by_key(|qc| qc.message.view)
            .unwrap();

        // Create the block proposal to send to the replicas,
        // and the commit vote to store in our block proposal cache.
        let (proposal, payload) = match highest_vote {
            // The previous block was not finalized, so we need to propose it again.
            // For this we only need the header, since we are guaranteed that at least
            // f+1 honest replicas have the block can broadcast when finalized
            // (2f+1 have stated that they voted for the block, at most f are malicious).
            Some(proposal) if proposal != highest_qc.message.proposal => (proposal, None),
            // The previous block was finalized, so we can propose a new block.
            _ => {
                // TODO(bruno): For now we just create a block with a random payload. After we integrate with
                //              the execution layer we should have a call here to the mempool to get a real payload.
                let mut payload = validator::Payload(vec![0; ConsensusInner::PAYLOAD_MAX_SIZE]);
                ctx.rng().fill(&mut payload.0[..]);

                metrics::METRICS
                    .leader_proposal_payload_size
                    .observe(payload.0.len());
                let proposal =
                    validator::BlockHeader::new(&highest_qc.message.proposal, payload.hash());
                (proposal, Some(payload))
            }
        };

        // ----------- Update the state machine --------------

        self.view = message.view;
        self.phase = validator::Phase::Commit;
        self.phase_start = ctx.now();
        self.block_proposal_cache = Some(proposal);

        // ----------- Prepare our message and send it --------------

        // Create the justification for our message.
        let justification = validator::PrepareQC::from(&replica_messages, &consensus.validator_set)
            .expect("Couldn't create justification from valid replica messages!");

        // Broadcast the leader prepare message to all replicas (ourselves included).
        let output_message = ConsensusInputMessage {
            message: consensus
                .secret_key
                .sign_msg(validator::ConsensusMsg::LeaderPrepare(
                    validator::LeaderPrepare {
                        protocol_version: validator::CURRENT_VERSION,
                        view: self.view,
                        proposal,
                        proposal_payload: payload,
                        justification,
                    },
                )),
            recipient: Target::Broadcast,
        };
        consensus.pipe.send(output_message.into());

        Ok(())
    }
}
