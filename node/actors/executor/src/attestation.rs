//! Module to publish attestations over batches.

use std::sync::Arc;

use anyhow::Context;
use zksync_concurrency::ctx;
use zksync_concurrency::time;
use zksync_consensus_network::gossip::BatchVotesPublisher;
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
        let mut latest_batch_number = self
            .batch_store
            .latest_batch_number(ctx)
            .await
            .context("latest_batch_number")?
            .unwrap_or_default();

        let mut earliest_batch_number = self
            .batch_store
            .earliest_batch_number_to_sign(ctx)
            .await
            .context("earliest_batch_number_to_sign")?
            .unwrap_or(latest_batch_number);

        loop {
            while earliest_batch_number <= latest_batch_number {
                // Try to get the next batch to sign; the commitment might not be available just yet.
                let Some(batch) = self
                    .batch_store
                    .batch_to_sign(ctx, earliest_batch_number)
                    .await
                    .context("batch_to_sign")?
                else {
                    break;
                };

                // The certificates might be collected out of order because of how gossip works,
                // in which case we can skip signing it.
                let has_batch_qc = self
                    .batch_store
                    .has_batch_qc(ctx, earliest_batch_number)
                    .await
                    .context("has_batch_qc")?;

                if !has_batch_qc {
                    // We only have to publish a vote once; future peers can pull it from the register.
                    self.publisher
                        .publish(attesters, &self.attester.key, batch)
                        .await
                        .context("publish")?;
                }

                earliest_batch_number = earliest_batch_number.next();
            }

            // Wait some time before we poll the database again to see if there is a new batch to sign.
            ctx.sleep(POLL_INTERVAL).await?;

            // Refresh the upper end of the range.
            latest_batch_number = self
                .batch_store
                .latest_batch_number(ctx)
                .await
                .context("latest_batch_number")?
                .unwrap_or(latest_batch_number)
        }
    }
}
