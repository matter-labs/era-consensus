//! Module to publish attestations over batches.

use crate::Attester;
use anyhow::Context;
use std::sync::Arc;
use zksync_concurrency::{ctx, sync, time};
use zksync_consensus_network::gossip::{
    AttestationStatusReceiver, AttestationStatusWatch, BatchVotesPublisher,
};
use zksync_consensus_roles::attester;
use zksync_consensus_storage::{BatchStore, BlockStore};

/// Polls the database for new batches to be signed and publishes them to the gossip channel.
pub(super) struct AttesterRunner {
    block_store: Arc<BlockStore>,
    batch_store: Arc<BatchStore>,
    attester: Attester,
    publisher: BatchVotesPublisher,
    status: AttestationStatusReceiver,
    poll_interval: time::Duration,
}

impl AttesterRunner {
    /// Create a new instance of a runner.
    pub(super) fn new(
        block_store: Arc<BlockStore>,
        batch_store: Arc<BatchStore>,
        attester: Attester,
        publisher: BatchVotesPublisher,
        status: AttestationStatusReceiver,
        poll_interval: time::Duration,
    ) -> Self {
        Self {
            block_store,
            batch_store,
            attester,
            publisher,
            status,
            poll_interval,
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

        // Subscribe starts as seen but we don't want to miss the first item.
        self.status.mark_changed();

        loop {
            let batch_number = sync::changed(ctx, &mut self.status)
                .await?
                .next_batch_to_attest;

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
                ctx.sleep(self.poll_interval).await?;
            }
        }
    }
}

/// An interface which is used by attesters and nodes collecting votes over gossip to determine
/// which is the next batch they are all supposed to be voting on, according to the main node.
///
/// This is a convenience interface to be used with the [AttestationStatusRunner].
#[async_trait::async_trait]
pub trait AttestationStatusClient: 'static + Send + Sync {
    /// Get the next batch number for which the main node expects a batch QC to be formed.
    ///
    /// The API might return an error while genesis is being created, which we represent with `None`
    /// here and mean that we'll have to try again later.
    async fn next_batch_to_attest(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<attester::BatchNumber>>;
}

/// Use an [AttestationStatusClient] to periodically poll the main node and update the [AttestationStatusWatch].
///
/// This is provided for convenience.
pub struct AttestationStatusRunner {
    status: Arc<AttestationStatusWatch>,
    client: Box<dyn AttestationStatusClient>,
    poll_interval: time::Duration,
}

impl AttestationStatusRunner {
    /// Create a new [AttestationStatusWatch] and an [AttestationStatusRunner] to poll the main node.
    ///
    /// It polls the [AttestationStatusClient] until it returns a value to initialize the status with.
    pub async fn init(
        ctx: &ctx::Ctx,
        client: Box<dyn AttestationStatusClient>,
        poll_interval: time::Duration,
    ) -> ctx::OrCanceled<(Arc<AttestationStatusWatch>, Self)> {
        let status = Arc::new(AttestationStatusWatch::new(attester::BatchNumber(0)));
        let mut runner = Self {
            status: status.clone(),
            client,
            poll_interval,
        };
        runner.poll_until_some(ctx).await?;
        Ok((status, runner))
    }

    /// Initialize an [AttestationStatusWatch] based on a [BatchStore] and return it along with the [AttestationStatusRunner].
    pub async fn init_from_store(
        ctx: &ctx::Ctx,
        store: Arc<BatchStore>,
        poll_interval: time::Duration,
    ) -> ctx::OrCanceled<(Arc<AttestationStatusWatch>, Self)> {
        Self::init(
            ctx,
            Box::new(LocalAttestationStatusClient(store)),
            poll_interval,
        )
        .await
    }

    /// Run the poll loop.
    pub async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let _ = self.poll_forever(ctx).await;
        Ok(())
    }

    /// Poll the client forever in a loop or until canceled.
    async fn poll_forever(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        loop {
            self.poll_until_some(ctx).await?;
            ctx.sleep(self.poll_interval).await?;
        }
    }

    /// Poll the client until some data is returned and write it into the status.
    async fn poll_until_some(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        loop {
            match self.client.next_batch_to_attest(ctx).await {
                Ok(Some(next_batch_to_attest)) => {
                    self.status.update(next_batch_to_attest).await;
                    return Ok(());
                }
                Ok(None) => {
                    tracing::info!("waiting for attestation status...")
                }
                Err(error) => {
                    tracing::error!(
                        ?error,
                        "failed to poll attestation status, retrying later..."
                    )
                }
            }
            ctx.sleep(self.poll_interval).await?;
        }
    }
}

/// Implement the attestation status for the main node by returning the next to vote on from the [BatchStore].
struct LocalAttestationStatusClient(Arc<BatchStore>);

#[async_trait::async_trait]
impl AttestationStatusClient for LocalAttestationStatusClient {
    async fn next_batch_to_attest(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<attester::BatchNumber>> {
        self.0.next_batch_to_attest(ctx).await
    }
}
