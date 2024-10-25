use super::StateMachine;
use std::cmp::max;
use zksync_concurrency::{ctx, error::Wrap as _};
use zksync_consensus_roles::validator;
use zksync_consensus_storage as storage;

impl StateMachine {
    /// Makes a justification (for a ReplicaNewView or a LeaderProposal) based on the current state.
    pub(crate) fn get_justification(&self) -> validator::ProposalJustification {
        // We need some QC in order to be able to create a justification.
        // In fact, it should be impossible to get here without a QC. Because
        // we only get here after starting a new view, which requires a QC.
        assert!(self.high_commit_qc.is_some() || self.high_timeout_qc.is_some());

        // We use the highest QC as the justification. If both have the same view, we use the CommitQC.
        if self.high_commit_qc.as_ref().map(|x| x.view())
            >= self.high_timeout_qc.as_ref().map(|x| &x.view)
        {
            validator::ProposalJustification::Commit(self.high_commit_qc.clone().unwrap())
        } else {
            validator::ProposalJustification::Timeout(self.high_timeout_qc.clone().unwrap())
        }
    }

    /// Processes a (already verified) CommitQC. It bumps the local high_commit_qc and if
    /// we have the proposal corresponding to this qc, we save the corresponding block to DB.
    pub(crate) async fn process_commit_qc(
        &mut self,
        ctx: &ctx::Ctx,
        qc: &validator::CommitQC,
    ) -> ctx::Result<()> {
        self.high_commit_qc = max(Some(qc.clone()), self.high_commit_qc.clone());
        self.save_block(ctx, qc).await.wrap("save_block()")
    }

    /// Tries to build a finalized block from the given CommitQC. We simply search our
    /// block proposal cache for the matching block, and if we find it we build the block.
    /// If this method succeeds, it sends the finalized block to the executor.
    #[tracing::instrument(level = "debug", skip_all)]
    pub(crate) async fn save_block(
        &mut self,
        ctx: &ctx::Ctx,
        commit_qc: &validator::CommitQC,
    ) -> ctx::Result<()> {
        let Some(cache) = self.block_proposal_cache.get(&commit_qc.header().number) else {
            return Ok(());
        };
        let Some(payload) = cache.get(&commit_qc.header().payload) else {
            return Ok(());
        };
        let block = validator::FinalBlock {
            payload: payload.clone(),
            justification: commit_qc.clone(),
        };

        tracing::info!(
            "Finalized block {}: {:#?}",
            block.header().number,
            block.header().payload,
        );
        self.config
            .block_store
            .queue_block(ctx, block.clone().into())
            .await?;

        // For availability, replica should not proceed until it stores the block persistently.
        // Rationale is that after save_block, there is start_new_view which prunes the
        // cache. Without persisting this block, if all replicas crash just after
        // start_new_view, the payload becomes unavailable.
        self.config
            .block_store
            .wait_until_persisted(ctx, block.header().number)
            .await?;

        let number_metric = &crate::metrics::METRICS.finalized_block_number;
        let current_number = number_metric.get();
        number_metric.set(current_number.max(block.header().number.0));

        Ok(())
    }

    /// Backups the replica state to DB.
    pub(crate) async fn backup_state(&self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        let mut proposals = vec![];
        for (number, payloads) in &self.block_proposal_cache {
            proposals.extend(payloads.values().map(|p| storage::Proposal {
                number: *number,
                payload: p.clone(),
            }));
        }
        let backup = storage::ReplicaState {
            view: self.view_number,
            phase: self.phase,
            high_vote: self.high_vote.clone(),
            high_commit_qc: self.high_commit_qc.clone(),
            high_timeout_qc: self.high_timeout_qc.clone(),
            proposals,
        };
        self.config
            .replica_store
            .set_state(ctx, &backup)
            .await
            .wrap("put_replica_state")?;
        Ok(())
    }
}
