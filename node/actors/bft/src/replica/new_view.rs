use super::StateMachine;
use crate::metrics;
use tracing::instrument;
use zksync_concurrency::{ctx, error::Wrap as _};
use zksync_consensus_network::io::{ConsensusInputMessage, Target};
use zksync_consensus_roles::validator;

impl StateMachine {
    /// This blocking method is used whenever we start a new view.
    #[instrument(level = "trace", err)]
    pub(crate) async fn start_new_view(&mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        // Update the state machine.
        self.view = self.view.next();
        tracing::info!("Starting view {}", self.view);
        metrics::METRICS.replica_view_number.set(self.view.0);

        self.phase = validator::Phase::Prepare;
        if let Some(qc) = self.high_qc.as_ref() {
            // Clear the block cache.
            self.block_proposal_cache
                .retain(|k, _| k > &qc.header().number);
        }

        // Backup our state.
        self.backup_state(ctx).await.wrap("backup_state()")?;

        // Send the replica message to the next leader.
        let output_message = ConsensusInputMessage {
            message: self
                .config
                .secret_key
                .sign_msg(validator::ConsensusMsg::ReplicaPrepare(
                    validator::ReplicaPrepare {
                        view: validator::View {
                            genesis: self.config.genesis().hash(),
                            number: self.view,
                        },
                        high_vote: self.high_vote.clone(),
                        high_qc: self.high_qc.clone(),
                    },
                )),
            recipient: Target::Broadcast,
        };
        self.outbound_pipe.send(output_message.into());

        // Reset the timer.
        self.reset_timer(ctx);
        Ok(())
    }
}
