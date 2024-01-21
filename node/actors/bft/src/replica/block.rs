use super::StateMachine;
use tracing::instrument;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;

impl StateMachine {
    /// Tries to build a finalized block from the given CommitQC. We simply search our
    /// block proposal cache for the matching block, and if we find it we build the block.
    /// If this method succeeds, it sends the finalized block to the executor.
    #[instrument(level = "debug", skip(self), ret)]
    pub(crate) async fn save_block(
        &mut self,
        ctx: &ctx::Ctx,
        commit_qc: &validator::CommitQC,
    ) -> ctx::Result<()> {
        // TODO(gprusak): for availability of finalized blocks,
        //                replicas should be able to broadcast highest quorums without
        //                the corresponding block (same goes for synchronization).
        let Some(cache) = self
            .block_proposal_cache
            .get(&commit_qc.message.proposal.number)
        else {
            return Ok(());
        };
        let Some(payload) = cache.get(&commit_qc.message.proposal.payload) else {
            return Ok(());
        };
        let block = validator::FinalBlock {
            payload: payload.clone(),
            justification: commit_qc.clone(),
        };

        tracing::info!(
            "Finalized block {}: {:#?}",
            block.header().number,
            block.header().hash(),
        );
        self.config
            .block_store
            .queue_block(ctx, block.clone())
            .await?;
        // For availability, replica should not proceed until it stores the block persistently.
        self.config
            .block_store
            .wait_until_persisted(ctx, block.header().number)
            .await?;

        let number_metric = &crate::metrics::METRICS.finalized_block_number;
        let current_number = number_metric.get();
        number_metric.set(current_number.max(block.header().number.0));
        Ok(())
    }
}
