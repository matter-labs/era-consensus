//! Module to publish attestations over batches.

use std::sync::Arc;

use anyhow::Context;
use zksync_concurrency::ctx;
use zksync_concurrency::time;
use zksync_consensus_network::gossip::BatchVotesPublisher;
use zksync_consensus_roles::attester::BatchNumber;
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
        // The first batch number we want to publish our vote for. We don't have to re-publish a vote
        // because once it enters the vote register even future peers can pull it from there.
        let mut min_batch_number = BatchNumber(0);
        loop {
            // Pretend that the attesters can evolve.
            let Some(attesters) = self.block_store.genesis().attesters.as_ref() else {
                continue;
            };
            if !attesters.contains(&public_key) {
                continue;
            }

            let mut unsigned_batch_numbers = self
                .batch_store
                .unsigned_batch_numbers(ctx)
                .await
                .context("unsigned_batch_numbers")?;

            // Just to be sure we go from smaller to higher batches.
            unsigned_batch_numbers.sort();

            for bn in unsigned_batch_numbers {
                // If we have already voted on this we can move on, no need to fetch the payload again.
                // Batches appear in the store in order, even if we might have QC for a newer and not for an older batch,
                // so once published our vote for a certain height, we can expect that we only have to vote on newer batches.
                if bn < min_batch_number {
                    continue;
                }

                if let Some(batch) = self
                    .batch_store
                    .batch_to_sign(ctx, bn)
                    .await
                    .context("batch_to_sign")?
                {
                    min_batch_number = batch.number.next();

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
