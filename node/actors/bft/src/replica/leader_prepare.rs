use super::StateMachine;
use std::collections::HashMap;
use tracing::instrument;
use zksync_concurrency::{ctx, error::Wrap};
use zksync_consensus_network::io::{ConsensusInputMessage, Target};
use zksync_consensus_roles::validator::{self, ProtocolVersion};

/// Errors that can occur when processing a "leader prepare" message.
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
    /// Invalid leader.
    #[error(
        "invalid leader (correct leader: {correct_leader:?}, received leader: {received_leader:?})"
    )]
    InvalidLeader {
        /// Correct leader.
        correct_leader: validator::PublicKey,
        /// Received leader.
        received_leader: validator::PublicKey,
    },
    /// Message for a past view or phase.
    #[error(
        "message for a past view / phase (current view: {current_view:?}, current phase: {current_phase:?})"
    )]
    Old {
        /// Current view.
        current_view: validator::ViewNumber,
        /// Current phase.
        current_phase: validator::Phase,
    },
    /// Invalid message signature.
    #[error("invalid signature: {0:#}")]
    InvalidSignature(#[source] validator::Error),
    /// Invalid message.
    #[error("invalid message: {0:#}")]
    InvalidMessage(#[source] anyhow::Error),
    /// Previous proposal was not finalized.
    #[error("new block proposal when the previous proposal was not finalized")]
    ProposalWhenPreviousNotFinalized,
    /// Invalid parent hash.
    #[error(
        "block proposal with invalid parent hash (correct parent hash: {correct_parent_hash:#?}, \
         received parent hash: {received_parent_hash:#?}, block: {header:?})"
    )]
    ProposalInvalidParentHash {
        /// Correct parent hash.
        correct_parent_hash: validator::BlockHeaderHash,
        /// Received parent hash.
        received_parent_hash: Option<validator::BlockHeaderHash>,
        /// Header including the incorrect parent hash.
        header: validator::BlockHeader,
    },
    /// Non-sequential proposal number.
    #[error(
        "block proposal with non-sequential number (correct proposal number: {correct_number}, \
         received proposal number: {received_number}, block: {header:?})"
    )]
    ProposalNonSequentialNumber {
        /// Correct proposal number.
        correct_number: validator::BlockNumber,
        /// Received proposal number.
        received_number: validator::BlockNumber,
        /// Header including the incorrect proposal number.
        header: validator::BlockHeader,
    },
    /// Mismatched payload.
    #[error("block proposal with mismatched payload")]
    ProposalMismatchedPayload,
    /// Oversized payload.
    #[error(
        "block proposal with an oversized payload (payload size: {payload_size}, block: {header:?}"
    )]
    ProposalOversizedPayload {
        /// Size of the payload.
        payload_size: usize,
        /// Proposal header corresponding to the payload.
        header: validator::BlockHeader,
    },
    /// Invalid payload.
    #[error("invalid payload: {0:#}")]
    ProposalInvalidPayload(#[source] anyhow::Error),
    /// Re-proposal without quorum.
    #[error("block re-proposal without quorum for the re-proposal")]
    ReproposalWithoutQuorum,
    /// Re-proposal when the previous proposal was finalized.
    #[error("block re-proposal when the previous proposal was finalized")]
    ReproposalWhenFinalized,
    /// Re-proposal of invalid block.
    #[error("block re-proposal of invalid block")]
    ReproposalInvalidBlock,
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
    /// Processes a leader prepare message.
    #[instrument(level = "trace", ret)]
    pub(crate) async fn process_leader_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        signed_message: validator::Signed<validator::LeaderPrepare>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;
        let view = message.view;

        // Check protocol version compatibility.
        if !crate::PROTOCOL_VERSION.compatible(&message.protocol_version) {
            return Err(Error::IncompatibleProtocolVersion {
                message_version: message.protocol_version,
                local_version: crate::PROTOCOL_VERSION,
            });
        }

        // Check that it comes from the correct leader.
        let leader = self.config.genesis.validators.view_leader(view);
        if author != &leader {
            return Err(Error::InvalidLeader {
                correct_leader: leader,
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
        
        // Check that the payload doesn't exceed the maximum size.
        if payload.0.len() > self.config.max_payload_size {
            return Err(Error::ProposalOversizedPayload {
                payload_size: payload.0.len(),
                header: message.proposal,
            });
        }

        // Verify the PrepareQC.
        message
            .verify(&self.config.genesis)
            .map_err(Error::InvalidMessage)?;


        // Try to create a finalized block with this CommitQC and our block proposal cache.
        // This gives us another chance to finalize a block that we may have missed before.
        self.save_block(ctx, &highest_qc)
            .await
            .wrap("save_block()")?;

        // ----------- Checking the block proposal --------------

        // Payload should be valid.
                // Defensively assume that PayloadManager cannot verify proposal until the previous block is stored.
                self.config
                    .block_store
                    .wait_until_persisted(ctx, highest_qc.header().number)
                    .await
                    .map_err(ctx::Error::Canceled)?;
                if let Err(err) = self
                    .config
                    .payload_manager
                    .verify(ctx, message.proposal.number, payload)
                    .await
                {
                    return Err(match err {
                        err @ ctx::Error::Canceled(_) => Error::Internal(err),
                        ctx::Error::Internal(err) => Error::ProposalInvalidPayload(err),
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
            protocol_version: crate::PROTOCOL_VERSION,
            view,
            proposal: message.proposal,
        };

        // Update the state machine.
        self.view = view;
        self.phase = validator::Phase::Commit;
        self.high_vote = Some(commit_vote.clone());

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
        self.backup_state(ctx).await.wrap("backup_state()")?;

        // Send the replica message to the leader.
        let output_message = ConsensusInputMessage {
            message: self
                .config
                .secret_key
                .sign_msg(validator::ConsensusMsg::ReplicaCommit(commit_vote)),
            recipient: Target::Validator(author.clone()),
        };
        self.outbound_pipe.send(output_message.into());

        Ok(())
    }
}
