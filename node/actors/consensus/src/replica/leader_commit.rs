use super::StateMachine;
use crate::{inner::ConsensusInner, replica::error::Error};

use concurrency::ctx;
use roles::validator;
use tracing::instrument;

impl StateMachine {
    /// Processes a leader commit message. We can approve this leader message even if we
    /// don't have the block proposal stored. It is enough to see the justification.
    #[instrument(level = "trace", ret)]
    pub(crate) fn process_leader_commit(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        signed_message: validator::Signed<validator::LeaderCommit>,
    ) -> Result<(), Error> {
        // ----------- Checking origin of the message --------------

        // Unwrap message.
        let message = &signed_message.msg;
        let author = &signed_message.key;
        let view = message.justification.message.view;

        // Check that it comes from the correct leader.
        if author != &consensus.view_leader(view) {
            return Err(Error::LeaderCommitInvalidLeader {
                correct_leader: consensus.view_leader(view),
                received_leader: author.clone(),
            });
        }

        // If the message is from the "past", we discard it.
        if (view, validator::Phase::Commit) < (self.view, self.phase) {
            return Err(Error::LeaderCommitOld {
                current_view: self.view,
                current_phase: self.phase,
            });
        }

        // ----------- Checking the signed part of the message --------------

        // Check the signature on the message.
        signed_message
            .verify()
            .map_err(Error::LeaderCommitInvalidSignature)?;

        // ----------- Checking the justification of the message --------------

        // Verify the QuorumCertificate.
        message
            .justification
            .verify(&consensus.validator_set, consensus.threshold())
            .map_err(Error::LeaderCommitInvalidJustification)?;

        // ----------- All checks finished. Now we process the message. --------------

        // Try to create a finalized block with this CommitQC and our block proposal cache.
        self.build_block(consensus, &message.justification);

        // Update the state machine. We don't update the view and phase (or backup our state) here
        // because we will do it when we start the new view.
        if message.justification.message.view >= self.high_qc.message.view {
            self.high_qc = message.justification.clone();
        }

        // Start a new view. But first we skip to the view of this message.
        self.view = view;
        self.start_new_view(ctx, consensus);

        Ok(())
    }
}
