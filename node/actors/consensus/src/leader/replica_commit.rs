use super::StateMachine;
use crate::{inner::ConsensusInner, leader::error::Error};
use anyhow::{anyhow, Context};
use concurrency::ctx;
use network::io::{ConsensusInputMessage, Target};
use once_cell::sync::Lazy;
use roles::validator;
use tracing::instrument;

static COMMIT_PHASE_LATENCY: Lazy<prometheus::Histogram> = Lazy::new(|| {
    prometheus::register_histogram!(
        "consensus_leader__commit_phase_latency",
        "latency of the commit phase observed by the leader",
        prometheus::exponential_buckets(0.01, 1.5, 20).unwrap(),
    )
    .unwrap()
});

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
            return Err(Error::Other(anyhow!("Received replica commit message for a past view/phase.\nCurrent view: {:?}\nCurrent phase: {:?}",
                self.view, self.phase)));
        }

        // If the message is for a view when we are not a leader, we discard it.
        if consensus.view_leader(message.view) != consensus.secret_key.public() {
            return Err(Error::Other(anyhow!(
                "Received replica commit message for a view when we are not a leader."
            )));
        }

        // If we already have a message from the same validator and for the same view, we discard it.
        if let Some(existing_message) = self
            .commit_message_cache
            .get(&message.view)
            .and_then(|x| x.get(author))
        {
            return Err(Error::Other(anyhow!(
                "Received replica commit message that we already have.\nExisting message: {:?}",
                existing_message
            )));
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message
            .verify()
            .with_context(|| "Received replica commit message with invalid signature.")?;

        // ----------- Checking the contents of the message --------------

        // We only accept replica commit messages for proposals that we have cached. That's so
        // we don't need to store replica commit messages for different proposals.
        if self.block_proposal_cache != Some(message) {
            return Err(Error::MissingProposal);
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
            return Err(Error::Other(anyhow!("Received replica commit message. Waiting for more messages.\nCurrent number of messages: {}\nThreshold: {}",
            num_messages,
            consensus.threshold())));
        }

        debug_assert!(num_messages == consensus.threshold());

        // ----------- Update the state machine --------------

        let now = ctx.now();
        COMMIT_PHASE_LATENCY.observe((now - self.phase_start).as_seconds_f64());
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
