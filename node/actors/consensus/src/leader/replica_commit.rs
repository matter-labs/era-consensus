use super::StateMachine;
use crate::{inner::ConsensusInner, metrics};
use concurrency::{ctx, metrics::LatencyHistogramExt as _};
use network::io::{ConsensusInputMessage, Target};
use roles::validator;
use tracing::instrument;

#[derive(thiserror::Error, Debug)]
#[allow(clippy::missing_docs_in_private_items)]
pub(crate) enum Error {
    #[error("unexpected proposal")]
    UnexpectedProposal,
    #[error("past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    Old {
        current_view: validator::ViewNumber,
        current_phase: validator::Phase,
    },
    #[error("we are not a leader for this message's view")]
    WhenNotLeaderInView,
    #[error("duplicate message from a replica (existing message: {existing_message:?}")]
    DuplicateMessage {
        existing_message: validator::ReplicaCommit,
    },
    #[error("number of received messages is below threshold. waiting for more (received: {num_messages:?}, threshold: {threshold:?}")]
    NumReceivedBelowThreshold {
        num_messages: usize,
        threshold: usize,
    },
    #[error("received replica commit message with invalid signature")]
    InvalidSignature(#[source] crypto::bls12_381::Error),
}

impl StateMachine {
    #[instrument(level = "trace", ret)]
    pub(crate) fn process_replica_commit(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        signed_message: validator::Signed<validator::ReplicaCommit>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = signed_message.msg;
        let author = &signed_message.key;

        // If the message is from the "past", we discard it.
        if (message.view, validator::Phase::Commit) < (self.view, self.phase) {
            return Err(Error::Old {
                current_view: self.view,
                current_phase: self.phase,
            });
        }

        // If the message is for a view when we are not a leader, we discard it.
        if consensus.view_leader(message.view) != consensus.secret_key.public() {
            return Err(Error::WhenNotLeaderInView);
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
        let num_messages = self.commit_message_cache.get(&message.view).unwrap().len();

        if num_messages < consensus.threshold() {
            return Err(Error::NumReceivedBelowThreshold {
                num_messages,
                threshold: consensus.threshold(),
            });
        }

        debug_assert!(num_messages == consensus.threshold());

        // ----------- Update the state machine --------------

        let now = ctx.now();
        metrics::METRICS
            .leader_commit_phase_latency
            .observe_latency(now - self.phase_start);
        self.view = message.view.next();
        self.phase = validator::Phase::Prepare;
        self.phase_start = now;

        // ----------- Prepare our message and send it. --------------

        // Get all the replica commit messages for this view. Note that we consume the
        // messages here. That's purposeful, so that we don't create a new leader commit
        // for this same view if we receive another replica commit message after this.
        let replica_messages = self
            .commit_message_cache
            .remove(&message.view)
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();

        // Create the justification for our message.
        let justification = validator::CommitQC::from(&replica_messages, &consensus.validator_set)
            .expect("Couldn't create justification from valid replica messages!");

        // Broadcast the leader commit message to all replicas (ourselves included).
        let output_message = ConsensusInputMessage {
            message: consensus
                .secret_key
                .sign_msg(validator::ConsensusMsg::LeaderCommit(
                    validator::LeaderCommit { justification },
                )),
            recipient: Target::Broadcast,
        };
        consensus.pipe.send(output_message.into());

        // Clean the caches.
        self.block_proposal_cache = None;
        self.prepare_message_cache.retain(|k, _| k >= &self.view);
        self.commit_message_cache.retain(|k, _| k >= &self.view);

        Ok(())
    }
}
