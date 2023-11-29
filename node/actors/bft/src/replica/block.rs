use super::StateMachine;
use crate::inner::ConsensusInner;
use anyhow::Context as _;
use tracing::{info, instrument};
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;

impl StateMachine {
    /// Tries to build a finalized block from the given CommitQC. We simply search our
    /// block proposal cache for the matching block, and if we find it we build the block.
    /// If this method succeeds, it sends the finalized block to the executor.
    #[instrument(level = "trace", ret)]
    pub(crate) async fn save_block(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        commit_qc: &validator::CommitQC,
    ) -> Result<(), ctx::Error> {
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
            header: commit_qc.message.proposal,
            payload: payload.clone(),
            justification: commit_qc.clone(),
        };

        info!(
            "Finalized a block!\nFinal block: {:#?}",
            block.header.hash()
        );
        self.storage
            .put_block(ctx, &block)
            .await
            .context("store.put_block()")?;

        let number_metric = &crate::metrics::METRICS.finalized_block_number;
        let current_number = number_metric.get();
        number_metric.set(current_number.max(block.header.number.0));
        Ok(())
    }
}
