use super::StateMachine;
use tracing::instrument;
use zksync_concurrency::{ctx, error::Wrap as _};
use zksync_consensus_network::io::{ConsensusInputMessage, Target};
use zksync_consensus_roles::validator;

impl StateMachine {
    /// This blocking method is used whenever we start a new view.
    #[instrument(level = "trace", err)]
    pub(crate) async fn start_new_view(&mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        tracing::info!("Starting view {}", self.view.next().0);

        // Update the state machine.
        let next_view = self.view.next();

        self.view = next_view;
        self.phase = validator::Phase::Prepare;

        // Clear the block cache.
        self.block_proposal_cache
            .retain(|k, _| k > &self.high_qc.message.proposal.number);

        // Backup our state.
        self.backup_state(ctx).await.wrap("backup_state()")?;

        // Send the replica message to the next leader.
        let output_message = ConsensusInputMessage {
            message: self
                .config
                .secret_key
                .sign_msg(validator::ConsensusMsg::ReplicaPrepare(
                    validator::ReplicaPrepare {
                        protocol_version: crate::PROTOCOL_VERSION,
                        view: next_view,
                        high_vote: self.high_vote,
                        high_qc: self.high_qc.clone(),
                    },
                )),
            recipient: Target::Validator(self.config.view_leader(next_view)),
        };
        self.outbound_pipe.send(output_message.into());

        // Reset the timer.
        self.reset_timer(ctx);
        Ok(())
    }
}
