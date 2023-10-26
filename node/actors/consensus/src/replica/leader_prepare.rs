use super::StateMachine;
use crate::inner::ConsensusInner;
use anyhow::Context as _;
use concurrency::ctx;
use network::io::{ConsensusInputMessage, Target};
use roles::validator;
use std::collections::HashMap;
use tracing::instrument;

#[derive(thiserror::Error, Debug)]
#[allow(clippy::missing_docs_in_private_items)]
pub(crate) enum Error {
    #[error("invalid leader (correct leader: {correct_leader:?}, received leader: {received_leader:?}])")]
    InvalidLeader {
        correct_leader: validator::PublicKey,
        received_leader: validator::PublicKey,
    },
    #[error("message for a past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    Old {
        current_view: validator::ViewNumber,
        current_phase: validator::Phase,
    },
    #[error("invalid signature: {0:#}")]
    InvalidSignature(#[source] crypto::bls12_381::Error),
    #[error("invalid PrepareQC: {0:#}")]
    InvalidPrepareQC(#[source] anyhow::Error),
    #[error("invalid high QC: {0:#}")]
    InvalidHighQC(#[source] anyhow::Error),
    #[error(
        "high QC of a future view (high QC view: {high_qc_view:?}, current view: {current_view:?}"
    )]
    HighQCOfFutureView {
        high_qc_view: validator::ViewNumber,
        current_view: validator::ViewNumber,
    },
    #[error("new block proposal when the previous proposal was not finalized")]
    ProposalWhenPreviousNotFinalized,
    #[error("block proposal with invalid parent hash (correct parent hash: {correct_parent_hash:#?}, received parent hash: {received_parent_hash:#?}, block: {header:?})")]
    ProposalInvalidParentHash {
        correct_parent_hash: validator::BlockHeaderHash,
        received_parent_hash: validator::BlockHeaderHash,
        header: validator::BlockHeader,
    },
    #[error("block proposal with non-sequential number (correct proposal number: {correct_number}, received proposal number: {received_number}, block: {header:?})")]
    ProposalNonSequentialNumber {
        correct_number: validator::BlockNumber,
        received_number: validator::BlockNumber,
        header: validator::BlockHeader,
    },
    #[error("block proposal with mismatched payload")]
    ProposalMismatchedPayload,
    #[error(
        "block proposal with an oversized payload (payload size: {payload_size}, block: {header:?}"
    )]
    ProposalOversizedPayload {
        payload_size: usize,
        header: validator::BlockHeader,
    },
    #[error("block re-proposal without quorum for the re-proposal")]
    ReproposalWithoutQuorum,
    #[error("block re-proposal when the previous proposal was finalized")]
    ReproposalWhenFinalized,
    #[error("block re-proposal of invalid block")]
    ReproposalInvalidBlock,
    #[error("internal error: {0:#}")]
    Internal(#[from] anyhow::Error),
}

impl StateMachine {
    /// Processes a leader prepare message.
    #[instrument(level = "trace", ret)]
    pub(crate) fn process_leader_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        signed_message: validator::Signed<validator::LeaderPrepare>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;
        let view = message.view;

        // Check that it comes from the correct leader.
        if author != &consensus.view_leader(view) {
            return Err(Error::InvalidLeader {
                correct_leader: consensus.view_leader(view),
                received_leader: author.clone(),
            });
        }

        // If the message is from the "past", we discard it.
        if (view, validator::Phase::Prepare) < (self.view, self.phase) {
            return Err(Error::Old {
                current_view: self.view,
                current_phase: self.phase,
            });
        }

        // ----------- Checking the signed part of the message --------------

        signed_message.verify().map_err(Error::InvalidSignature)?;

        // ----------- Checking the justification of the message --------------

        // Verify the PrepareQC.
        message
            .justification
            .verify(view, &consensus.validator_set, consensus.threshold())
            .map_err(Error::InvalidPrepareQC)?;

        // Get the highest block voted and check if there's a quorum of votes for it. To have a quorum
        // in this situation, we require 2*f+1 votes, where f is the maximum number of faulty replicas.
        let mut vote_count: HashMap<_, usize> = HashMap::new();

        for (msg, signers) in &message.justification.map {
            *vote_count.entry(msg.high_vote.proposal).or_default() += signers.len();
        }

        let highest_vote: Option<validator::BlockHeader> = vote_count
            .into_iter()
            // We only take one value from the iterator because there can only be at most one block with a quorum of 2f+1 votes.
            .find(|(_, v)| *v > 2 * consensus.faulty_replicas())
            .map(|(h, _)| h);

        // Get the highest CommitQC and verify it.
        let highest_qc: validator::CommitQC = message
            .justification
            .map
            .keys()
            .max_by_key(|m| m.high_qc.message.view)
            .unwrap()
            .high_qc
            .clone();

        highest_qc
            .verify(&consensus.validator_set, consensus.threshold())
            .map_err(Error::InvalidHighQC)?;

        // If the high QC is for a future view, we discard the message.
        // This check is not necessary for correctness, but it's useful to
        // guarantee that our messages don't contain QCs from the future.
        if highest_qc.message.view >= view {
            return Err(Error::HighQCOfFutureView {
                high_qc_view: highest_qc.message.view,
                current_view: view,
            });
        }

        // Try to create a finalized block with this CommitQC and our block proposal cache.
        // This gives us another chance to finalize a block that we may have missed before.
        self.build_block(consensus, &highest_qc);

        // ----------- Checking the block proposal --------------

        // Check that the proposal is valid.
        match &message.proposal_payload {
            // The leader proposed a new block.
            Some(payload) => {
                // Check that the payload doesn't exceed the maximum size.
                if payload.0.len() > ConsensusInner::PAYLOAD_MAX_SIZE {
                    return Err(Error::ProposalOversizedPayload {
                        payload_size: payload.0.len(),
                        header: message.proposal,
                    });
                }

                // Check that payload matches the header
                if message.proposal.payload != payload.hash() {
                    return Err(Error::ProposalMismatchedPayload);
                }

                // Check that we finalized the previous block.
                if highest_vote.is_some()
                    && highest_vote.as_ref() != Some(&highest_qc.message.proposal)
                {
                    return Err(Error::ProposalWhenPreviousNotFinalized);
                }

                // Parent hash should match.
                if highest_qc.message.proposal.hash() != message.proposal.parent {
                    return Err(Error::ProposalInvalidParentHash {
                        correct_parent_hash: highest_qc.message.proposal.hash(),
                        received_parent_hash: message.proposal.parent,
                        header: message.proposal,
                    });
                }

                // Block number should match.
                if highest_qc.message.proposal.number.next() != message.proposal.number {
                    return Err(Error::ProposalNonSequentialNumber {
                        correct_number: highest_qc.message.proposal.number.next(),
                        received_number: message.proposal.number,
                        header: message.proposal,
                    });
                }
            }
            // The leader is re-proposing a past block.
            None => {
                let Some(highest_vote) = highest_vote else {
                    return Err(Error::ReproposalWithoutQuorum);
                };
                if highest_vote == highest_qc.message.proposal {
                    return Err(Error::ReproposalWhenFinalized);
                }
                if highest_vote != message.proposal {
                    return Err(Error::ReproposalInvalidBlock);
                }
            }
        }

        // ----------- All checks finished. Now we process the message. --------------

        // Create our commit vote.
        let commit_vote = validator::ReplicaCommit {
            protocol_version: validator::CURRENT_VERSION,
            view,
            proposal: message.proposal,
        };

        // Update the state machine.
        self.view = view;
        self.phase = validator::Phase::Commit;
        self.high_vote = commit_vote;

        if highest_qc.message.view > self.high_qc.message.view {
            self.high_qc = highest_qc;
        }

        // If we received a new block proposal, store it in our cache.
        if let Some(payload) = &message.proposal_payload {
            self.block_proposal_cache
                .entry(message.proposal.number)
                .or_default()
                .insert(payload.hash(), payload.clone());
        }

        // Backup our state.
        self.backup_state(ctx).context("backup_state()")?;

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
