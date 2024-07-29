//! Module to publish attestations over batches.

use crate::Attester;
use anyhow::Context;
use std::sync::Arc;
use zksync_concurrency::{ctx, sync, time};
use zksync_consensus_network::gossip::{AttestationStatusReceiver, BatchVotesPublisher};
use zksync_consensus_roles::attester;
use zksync_consensus_storage::{BatchStore, BlockStore};

const POLL_INTERVAL: time::Duration = time::Duration::seconds(1);

/// Polls the database for new batches to be signed and publishes them to the gossip channel.
pub(super) struct AttesterRunner {
    block_store: Arc<BlockStore>,
    batch_store: Arc<BatchStore>,
    attester: Attester,
    publisher: BatchVotesPublisher,
    status: AttestationStatusReceiver,
}

impl AttesterRunner {
    /// Create a new instance of a runner.
    pub(super) fn new(
        block_store: Arc<BlockStore>,
        batch_store: Arc<BatchStore>,
        attester: Attester,
        publisher: BatchVotesPublisher,
        status: AttestationStatusReceiver,
    ) -> Self {
        Self {
            block_store,
            batch_store,
            attester,
            publisher,
            status,
        }
    }
    /// Poll the database for new L1 batches and publish our signature over the batch.
    pub(super) async fn run(mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
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

        let genesis = self.block_store.genesis().hash();

        let mut prev = None;

        loop {
            let batch_number =
                sync::wait_for_some(ctx, &mut self.status, |s| match s.next_batch_to_attest {
                    next if next == prev => None,
                    next => next,
                })
                .await?;

            tracing::info!(%batch_number, "attestation status");

            // We can avoid actively polling the database in `wait_for_batch_to_sign` by waiting its existence
            // to be indicated in memory (which itself relies on polling). This happens once we have the commitment,
            // which for nodes that get the blocks through BFT should happen after execution. Nodes which
            // rely on batch sync don't participate in attestations as they need the batch on L1 first.
            self.batch_store
                .wait_until_persisted(ctx, batch_number)
                .await?;

            // Try to get the next batch to sign; the commitment might not be available just yet.
            let batch = self.wait_for_batch_to_sign(ctx, batch_number).await?;

            // The certificates might be collected out of order because of how gossip works;
            // we could query the DB to see if we already have a QC, or we can just go ahead
            // and publish our vote, and let others ignore it.

            tracing::info!(%batch_number, "publishing attestation");

            // We only have to publish a vote once; future peers can pull it from the register.
            self.publisher
                .publish(attesters, &genesis, &self.attester.key, batch)
                .await
                .context("publish")?;

            prev = Some(batch_number);
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
