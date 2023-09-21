use super::StateMachine;
use crate::ConsensusInner;
use concurrency::ctx;
use network::io::{ConsensusInputMessage, Target};
use roles::validator;
use tracing::{info, instrument};

impl StateMachine {
    /// This method is used whenever we start a new view.
    #[instrument(level = "trace", ret)]
    pub(crate) fn start_new_view(&mut self, ctx: &ctx::Ctx, consensus: &ConsensusInner) {
        info!("Starting view {}", self.view.next().0);

        // Update the state machine.
        let next_view = self.view.next();

        self.view = next_view;
        self.phase = validator::Phase::Prepare;

        // Clear the block cache.
        self.block_proposal_cache
            .retain(|k, _| k > &self.high_qc.message.proposal_block_number);

        // Backup our state.
        self.backup_state();

        // Send the replica message to the next leader.
        let output_message = ConsensusInputMessage {
            message: consensus
                .secret_key
                .sign_msg(validator::ConsensusMsg::ReplicaPrepare(
                    validator::ReplicaPrepare {
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
    }
}
