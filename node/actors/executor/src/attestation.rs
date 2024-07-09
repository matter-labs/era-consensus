//! Module to publish attestations over batches.

use std::sync::Arc;

use anyhow::Context;
use zksync_concurrency::ctx;
use zksync_concurrency::time;
use zksync_consensus_network::gossip::BatchVotesPublisher;
use zksync_consensus_roles::attester;
use zksync_consensus_storage::{BatchStore, BlockStore};

use crate::Attester;

const POLL_INTERVAL: time::Duration = time::Duration::seconds(1);

/// Polls the database for new batches to be signed and publishes them to the gossip channel.
pub(super) struct AttesterRunner {
    block_store: Arc<BlockStore>,
    batch_store: Arc<BatchStore>,
    attester: Attester,
    publisher: BatchVotesPublisher,
}

impl AttesterRunner {
    /// Create a new instance of a runner.
    pub(super) fn new(
        block_store: Arc<BlockStore>,
        batch_store: Arc<BatchStore>,
        attester: Attester,
        publisher: BatchVotesPublisher,
    ) -> Self {
        Self {
            block_store,
            batch_store,
            attester,
            publisher,
        }
    }
    /// Poll the database for new L1 batches and publish our signature over the batch.
    pub(super) async fn run(self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        let public_key = self.attester.key.public();
        // TODO: In the future when we have attester rotation these checks will have to be checked inside the loop.
        let Some(attesters) = self.block_store.genesis().attesters.as_ref() else {
            tracing::warn!("Attester key is set, but the attester committee is empty.");
            return Ok(());
        };
        if !attesters.contains(&public_key) {
            tracing::warn!("Attester key is set, but not part of the attester committee.");
            return Ok(());
        }

        // Find the initial range of batches that we want to (re)sign after a (re)start.
        let last_batch_number = self
            .batch_store
            .last_batch_number(ctx)
            .await
            .context("last_batch_number")?
            .unwrap_or_default();

        // Determine the batch to start signing from.
        let earliest_batch_number = self
            .batch_store
            .earliest_batch_number_to_sign(ctx)
            .await
            .context("earliest_batch_number_to_sign")?
            .unwrap_or(last_batch_number);

        tracing::info!(%earliest_batch_number, %last_batch_number, "attesting batches");

        let mut batch_number = earliest_batch_number;

        loop {
            // Try to get the next batch to sign; the commitment might not be available just yet.
            let batch = self.wait_for_batch_to_sign(ctx, batch_number).await?;

            // The certificates might be collected out of order because of how gossip works;
            // we could query the DB to see if we already have a QC, or we can just go ahead
            // and publish our vote, and let others ignore it.

            tracing::info!(%batch_number, "publishing attestation");

            // We only have to publish a vote once; future peers can pull it from the register.
            self.publisher
                .publish(attesters, &self.attester.key, batch)
                .await
                .context("publish")?;

            batch_number = batch_number.next();
        }
    }

    /// Wait for the batch commitment to become available.
    async fn wait_for_batch_to_sign(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<attester::Batch> {
        loop {
            if let Some(batch) = self
                .batch_store
                .batch_to_sign(ctx, number)
                .await
                .context("batch_to_sign")?
            {
                return Ok(batch);
            } else {
                ctx.sleep(POLL_INTERVAL).await?;
            }
        }
    }
}
