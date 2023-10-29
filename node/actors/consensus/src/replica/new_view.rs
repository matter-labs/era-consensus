use super::StateMachine;
use crate::ConsensusInner;
use anyhow::Context as _;
use concurrency::ctx;
use network::io::{ConsensusInputMessage, Target};
use roles::validator;
use tracing::instrument;

impl StateMachine {
    /// This blocking method is used whenever we start a new view.
    #[instrument(level = "trace", err)]
    pub(crate) fn start_new_view(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
    ) -> anyhow::Result<()> {
        tracing::info!("Starting view {}", self.view.next().0);

        // Update the state machine.
        let next_view = self.view.next();

        self.view = next_view;
        self.phase = validator::Phase::Prepare;

        // Clear the block cache.
        self.block_proposal_cache
            .retain(|k, _| k > &self.high_qc.message.proposal.number);

        // Backup our state.
        self.backup_state(ctx).context("backup_state")?;

        // Send the replica message to the next leader.
        let output_message = ConsensusInputMessage {
            message: consensus
                .secret_key
                .sign_msg(validator::ConsensusMsg::ReplicaPrepare(
                    validator::ReplicaPrepare {
                        protocol_version: validator::CURRENT_VERSION,
                        view: next_view,
                        high_vote: self.high_vote,
                        high_qc: self.high_qc.clone(),
                    },
                )),
            recipient: Target::Validator(consensus.view_leader(next_view)),
        };
        consensus.pipe.send(output_message.into());

        // Reset the timer.
        self.reset_timer(ctx);
        Ok(())
    }
}
