use super::StateMachine;
use crate::metrics;
use zksync_concurrency::{ctx, error::Wrap, metrics::LatencyHistogramExt as _};
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator::{self, BlockHeader, BlockNumber};

/// Errors that can occur when processing a LeaderProposal message.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
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
    /// Invalid message signature.
    #[error("invalid signature: {0:#}")]
    InvalidSignature(#[source] anyhow::Error),
    /// Invalid message.
    #[error("invalid message: {0:#}")]
    InvalidMessage(#[source] validator::LeaderProposalVerifyError),
    /// Leader proposed a block that was already pruned from replica's storage.
    #[error("leader proposed a block that was already pruned from replica's storage")]
    ProposalAlreadyPruned,
    /// Reproposal with an unnecessary payload.
    #[error("reproposal with an unnecessary payload")]
    ReproposalWithPayload,
    /// Block proposal payload missing.
    #[error("block proposal payload missing")]
    MissingPayload,
    /// Oversized payload.
    #[error("block proposal with an oversized payload (payload size: {payload_size})")]
    ProposalOversizedPayload {
        /// Size of the payload.
        payload_size: usize,
    },
    /// Previous payload missing.
    #[error("previous block proposal payload missing from store (block number: {prev_number})")]
    MissingPreviousPayload {
        /// The number of the missing block
        prev_number: BlockNumber,
    },
    /// Invalid payload.
    #[error("invalid payload: {0:#}")]
    InvalidPayload(#[source] anyhow::Error),
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
    /// Processes a LeaderProposal message.
    pub(crate) async fn on_proposal(
        &mut self,
        ctx: &ctx::Ctx,
        signed_message: validator::Signed<validator::LeaderProposal>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;
        let view = message.view().number;

        // Check that the message is for the current view or a future view. We only allow proposals for
        // the current view if we have not voted or timed out yet.
        if view < self.view_number
            || (view == self.view_number && self.phase != validator::Phase::Prepare)
        {
            return Err(Error::Old {
                current_view: self.view_number,
                current_phase: self.phase,
            });
        }

        // Check that it comes from the correct leader.
        let leader = self.config.genesis().view_leader(view);
        if author != &leader {
            return Err(Error::InvalidLeader {
                correct_leader: leader,
                received_leader: author.clone(),
            });
        }

        // ----------- Checking the message --------------

        signed_message.verify().map_err(Error::InvalidSignature)?;

        message
            .verify(self.config.genesis())
            .map_err(Error::InvalidMessage)?;

        let (implied_block_number, implied_block_hash) = message
            .justification
            .get_implied_block(self.config.genesis());

        // Replica MUSTN'T vote for blocks which have been already pruned for storage.
        // (because it won't be able to persist and broadcast them once finalized).
        // TODO(gprusak): it should never happen, we should add safety checks to prevent
        // pruning blocks not known to be finalized.
        if implied_block_number < self.config.block_store.queued().first {
            return Err(Error::ProposalAlreadyPruned);
        }

        let block_hash = match implied_block_hash {
            // This is a reproposal.
            // We let the leader repropose blocks without sending them in the proposal
            // (it sends only the block number + block hash). That allows a leader to
            // repropose a block without having it stored. Sending reproposals without
            // a payload is an optimization that allows us to not wait for a leader that
            // has the previous proposal stored (which can take 4f views), and to somewhat
            // speed up reproposals by skipping block broadcast.
            // This only saves time because we have a gossip network running in parallel,
            // and any time a replica is able to create a finalized block (by possessing
            // both the block and the commit QC) it broadcasts the finalized block (this
            // was meant to propagate the block to full nodes, but of course validators
            // will end up receiving it as well).
            Some(hash) => {
                // We check that the leader didn't send a payload with the reproposal.
                // This isn't technically needed for the consensus to work (it will remain
                // safe and live), but it's a good practice to avoid unnecessary data in
                // blockchain.
                // This unnecessary payload would also effectively be a source of free
                // data availability, which the leaders would be incentivized to abuse.
                if message.proposal_payload.is_some() {
                    return Err(Error::ReproposalWithPayload);
                };

                hash
            }
            // This is a new proposal, so we need to verify it (i.e. execute it).
            None => {
                // Check that the payload is present.
                let Some(ref payload) = message.proposal_payload else {
                    return Err(Error::MissingPayload);
                };

                if payload.len() > self.config.max_payload_size {
                    return Err(Error::ProposalOversizedPayload {
                        payload_size: payload.len(),
                    });
                }

                // Defensively assume that PayloadManager cannot verify proposal until the previous block is stored.
                // Note that it doesn't mean that the block is actually available, as old blocks might get pruned or
                // we might just have started from a snapshot state. It just means that we have the state of the chain
                // up to the previous block.
                if let Some(prev) = implied_block_number.prev() {
                    self.config
                        .block_store
                        .wait_until_persisted(&ctx.with_deadline(self.view_timeout), prev)
                        .await
                        .map_err(|_| Error::MissingPreviousPayload { prev_number: prev })?;
                }

                // Execute the payload.
                if let Err(err) = self
                    .config
                    .payload_manager
                    .verify(ctx, implied_block_number, payload)
                    .await
                {
                    return Err(match err {
                        ctx::Error::Internal(err) => Error::InvalidPayload(err),
                        err @ ctx::Error::Canceled(_) => Error::Internal(err),
                    });
                }

                // The proposal is valid. We cache it, waiting for it to be committed.
                self.block_proposal_cache
                    .entry(implied_block_number)
                    .or_default()
                    .insert(payload.hash(), payload.clone());

                payload.hash()
            }
        };

        // ----------- All checks finished. Now we process the message. --------------

        // Metrics. We observe the latency of receiving a proposal measured
        // from the start of this view.
        metrics::METRICS
            .proposal_latency
            .observe_latency(ctx.now() - self.view_start);

        // Create our commit vote.
        let commit_vote = validator::ReplicaCommit {
            view: message.view(),
            proposal: BlockHeader {
                number: implied_block_number,
                payload: block_hash,
            },
        };

        // Update the state machine.
        self.view_number = message.view().number;
        metrics::METRICS.replica_view_number.set(self.view_number.0);
        self.phase = validator::Phase::Commit;
        self.high_vote = Some(commit_vote.clone());
        match &message.justification {
            validator::ProposalJustification::Commit(qc) => self
                .process_commit_qc(ctx, qc)
                .await
                .wrap("process_commit_qc()")?,
            validator::ProposalJustification::Timeout(qc) => {
                if let Some(high_qc) = qc.high_qc() {
                    self.process_commit_qc(ctx, high_qc)
                        .await
                        .wrap("process_commit_qc()")?;
                }
                if self
                    .high_timeout_qc
                    .as_ref()
                    .map_or(true, |old| old.view.number < qc.view.number)
                {
                    self.high_timeout_qc = Some(qc.clone());
                }
            }
        };

        // Backup our state.
        self.backup_state(ctx).await.wrap("backup_state()")?;

        // Broadcast our commit message.
        let output_message = ConsensusInputMessage {
            message: self
                .config
                .secret_key
                .sign_msg(validator::ConsensusMsg::ReplicaCommit(commit_vote)),
        };
        self.outbound_channel.send(output_message);

        Ok(())
    }
}
