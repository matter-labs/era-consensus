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
    /// Crete a new instance of a runner.
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
        loop {
            // Pretend that the attesters can evolve.
            let Some(attesters) = self.block_store.genesis().attesters.as_ref() else {
                continue;
            };
            if !attesters.contains(&public_key) {
                continue;
            }

            let unsigned_batch_numbers = self
                .batch_store
                .unsigned_batch_numbers(ctx)
                .await
                .context("unsigned_batch_numbers")?;

            for bn in unsigned_batch_numbers {
                if let Some(batch) = self
                    .batch_store
                    .batch_to_sign(ctx, bn)
                    .await
                    .context("batch_to_sign")?
                {
                    self.publisher
                        .publish(attesters, &self.attester.key, batch)
                        .await
                        .context("publish")?;
                }
            }

            ctx.sleep(POLL_INTERVAL).await?;
        }
    }
}
