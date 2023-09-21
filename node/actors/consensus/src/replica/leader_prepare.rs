use super::StateMachine;
use crate::ConsensusInner;
use anyhow::{bail, Context};
use concurrency::ctx;
use network::io::{ConsensusInputMessage, Target};
use roles::validator;
use std::collections::HashMap;
use tracing::instrument;

impl StateMachine {
    /// Processes a leader prepare message.
    #[instrument(level = "trace", ret)]
    pub(crate) fn process_leader_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        signed_message: validator::Signed<validator::LeaderPrepare>,
    ) -> anyhow::Result<()> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;
        let view = message
            .justification
            .map
            .first_key_value()
            .context("Received leader prepare message with empty map in the justification.")?
            .0
            .view;

        // Check that it comes from the correct leader.
        if author != &consensus.view_leader(view) {
            bail!(
                "Received leader prepare message with invalid leader.\nCorrect leader: {:?}\nReceived leader: {:?}",
                consensus.view_leader(view), author
            );
        }

        // If the message is from the "past", we discard it.
        if (view, validator::Phase::Prepare) < (self.view, self.phase) {
            bail!(
            "Received leader prepare message for a past view/phase.\nCurrent view: {:?}\nCurrent phase: {:?}",
            self.view, self.phase
        );
        }

        // ----------- Checking the signed part of the message --------------

        signed_message
            .verify()
            .with_context(|| "Received leader prepare message with invalid signature.")?;

        // ----------- Checking the justification of the message --------------

        // Verify the PrepareQC.
        message
            .justification
            .verify(&consensus.validator_set, consensus.threshold())
            .with_context(|| "Received leader prepare message with invalid PrepareQC.")?;

        // Get the highest block voted and check if there's a quorum of votes for it. To have a quorum
        // in this situation, we require 2*f+1 votes, where f is the maximum number of faulty replicas.
        let mut vote_count: HashMap<_, usize> = HashMap::new();

        for (msg, signers) in &message.justification.map {
            *vote_count
                .entry((
                    msg.high_vote.proposal_block_number,
                    msg.high_vote.proposal_block_hash,
                ))
                .or_default() += signers.len();
        }

        let highest_vote = vote_count
            .iter()
            // We only take one value from the iterator because there can only be at most one block with a quorum of 2f+1 votes.
            .find(|(_, v)| **v > 2 * consensus.faulty_replicas())
            .map(|(h, _)| h);

        // Get the highest CommitQC and verify it.
        let highest_qc = message
            .justification
            .map
            .keys()
            .max_by_key(|m| m.high_qc.message.view)
            .unwrap()
            .high_qc
            .clone();

        highest_qc
            .verify(&consensus.validator_set, consensus.threshold())
            .with_context(|| "Received leader prepare message with invalid highest QC.")?;

        // If the high QC is for a future view, we discard the message.
        // This check is not necessary for correctness, but it's useful to
        // guarantee that our messages don't contain QCs from the future.
        if highest_qc.message.view >= view {
            bail!(
                "Received leader prepare message with a highest QC from the future.\nHighest QC view: {:?}\nCurrent view: {:?}",
                highest_qc.message.view, view
            );
        }

        // Try to create a finalized block with this CommitQC and our block proposal cache.
        // This gives us another chance to finalize a block that we may have missed before.
        self.build_block(consensus, &highest_qc);

        // ----------- Checking the block proposal --------------

        // Check that the proposal is valid.
        let (proposal_block_number, proposal_block_hash, proposal_block) = match &message.proposal {
            // The leader proposed a new block.
            validator::Proposal::New(block) => {
                // Check that we finalized the previous block.
                if highest_vote.is_some()
                    && highest_vote
                        != Some(&(
                            highest_qc.message.proposal_block_number,
                            highest_qc.message.proposal_block_hash,
                        ))
                {
                    bail!(
                        "Received new block proposal when the previous proposal was not finalized."
                    );
                }

                if highest_qc.message.proposal_block_hash != block.parent {
                    bail!(
                        "Received block proposal with invalid parent hash.\nCorrect parent hash: {:#?}\nReceived parent hash: {:#?}\nBlock: {:?}",
                        highest_qc.message.proposal_block_hash, block.parent, block
                    );
                }

                if highest_qc.message.proposal_block_number.next() != block.number {
                    bail!(
                        "Received block proposal with non-sequential number.\nCorrect proposal number: {}\nReceived proposal number: {}\nBlock: {:?}",
                        highest_qc.message.proposal_block_number.next().0, block.number.0, block
                    );
                }

                // Check that the payload doesn't exceed the maximum size.
                if block.payload.len() > ConsensusInner::PAYLOAD_MAX_SIZE {
                    bail!(
                        "Received block proposal with too large payload.\nPayload size: {}\nBlock: {:?}",
                        block.payload.len(),
                         block
                    );
                }

                (block.number, block.hash(), Some(block))
            }
            // The leader is re-proposing a past block.
            validator::Proposal::Retry(commit_vote) => {
                if highest_vote.is_none()
                    || highest_vote
                        == Some(&(
                            highest_qc.message.proposal_block_number,
                            highest_qc.message.proposal_block_hash,
                        ))
                {
                    bail!("Received block reproposal when the previous proposal was finalized.");
                }

                if highest_vote.unwrap()
                    != &(
                        commit_vote.proposal_block_number,
                        commit_vote.proposal_block_hash,
                    )
                {
                    bail!("Received invalid block re-proposal.",);
                }

                (highest_vote.unwrap().0, highest_vote.unwrap().1, None)
            }
        };

        // ----------- All checks finished. Now we process the message. --------------

        // Create our commit vote.
        let commit_vote = validator::ReplicaCommit {
            view,
            proposal_block_hash,
            proposal_block_number,
        };

        // Update the state machine.
        self.view = view;
        self.phase = validator::Phase::Commit;
        self.high_vote = commit_vote;

        if highest_qc.message.view > self.high_qc.message.view {
            self.high_qc = highest_qc;
        }

        // If we received a new block proposal, store it in our cache.
        if let Some(block) = proposal_block {
            match self.block_proposal_cache.get_mut(&proposal_block_number) {
                Some(map) => {
                    map.insert(proposal_block_hash, block.clone());
                }
                None => {
                    let mut map = HashMap::new();
                    map.insert(proposal_block_hash, block.clone());
                    self.block_proposal_cache.insert(proposal_block_number, map);
                }
            }
        }

        // Backup our state.
        self.backup_state();

        // Send the replica message to the leader.
        let output_message = ConsensusInputMessage {
            message: consensus
                .secret_key
                .sign_msg(validator::ConsensusMsg::ReplicaCommit(commit_vote)),
            recipient: Target::Validator(author.clone()),
        };
        consensus.pipe.send(output_message.into());

        Ok(())
    }
}
