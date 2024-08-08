//! Module to publish attestations over batches.

use crate::Attester;
use anyhow::Context;
use std::sync::Arc;
use tracing::Instrument;
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
            async {
                let Some(batch_number) = sync::changed(ctx, &mut self.status)
                    .instrument(tracing::info_span!("wait_for_attestation_status"))
                    .await?
                    .next_batch_to_attest
                else {
                    return Ok(());
                };

                tracing::info!(%batch_number, "attestation status");

                // We can avoid actively polling the database in `wait_for_batch_to_sign` by waiting its existence
                // to be indicated in memory (which itself relies on polling). This happens once we have the commitment,
                // which for nodes that get the blocks through BFT should happen after execution. Nodes which
                // rely on batch sync don't participate in attestations as they need the batch on L1 first.
                self.batch_store
                    .wait_until_persisted(ctx, batch_number)
                    .await?;

                // Try to get the next batch to sign; the commitment might not be available just yet.
                let batch = AttesterRunner::wait_for_batch_to_sign(
                    ctx,
                    batch_number,
                    &self.batch_store,
                    self.poll_interval,
                )
                .await?;

                // The certificates might be collected out of order because of how gossip works;
                // we could query the DB to see if we already have a QC, or we can just go ahead
                // and publish our vote, and let others ignore it.

                tracing::info!(%batch_number, "publishing attestation");

                // We only have to publish a vote once; future peers can pull it from the register.
                self.publisher
                    .publish(attesters, &genesis, &self.attester.key, batch)
                    .await
                    .context("publish")?;

                ctx::Ok(())
            }
            .instrument(tracing::info_span!("attestation_iter"))
            .await?;
        }
    }

    /// Wait for the batch commitment to become available.
    #[tracing::instrument(skip_all, fields(l1_batch = %number))]
    async fn wait_for_batch_to_sign(
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
        batch_store: &BatchStore,
        poll_interval: time::Duration,
    ) -> ctx::Result<attester::Batch> {
        loop {
            if let Some(batch) = batch_store
                .batch_to_sign(ctx, number)
                .await
                .context("batch_to_sign")?
            {
                return Ok(batch);
            } else {
                ctx.sleep(poll_interval).await?;
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
    ///
    /// The genesis hash is returned along with the new batch number to facilitate detecting reorgs
    /// on the main node as soon as possible and prevent inconsistent state from entering the system.
    async fn attestation_status(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<(attester::GenesisHash, attester::BatchNumber)>>;
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
        _ctx: &ctx::Ctx,
        client: Box<dyn AttestationStatusClient>,
        poll_interval: time::Duration,
        genesis: attester::GenesisHash,
    ) -> ctx::Result<(Arc<AttestationStatusWatch>, Self)> {
        let status = Arc::new(AttestationStatusWatch::new(genesis));
        let runner = Self {
            status: status.clone(),
            client,
            poll_interval,
        };
        // This would initialise the status to some value, however the EN was rolled out first without the main node API.
        // runner.poll_until_some(ctx).await?;
        Ok((status, runner))
    }

    /// Initialize an [AttestationStatusWatch] based on a [BatchStore] and return it along with the [AttestationStatusRunner].
    pub async fn init_from_store(
        ctx: &ctx::Ctx,
        batch_store: Arc<BatchStore>,
        poll_interval: time::Duration,
        genesis: attester::GenesisHash,
    ) -> ctx::Result<(Arc<AttestationStatusWatch>, Self)> {
        Self::init(
            ctx,
            Box::new(LocalAttestationStatusClient {
                genesis,
                batch_store,
            }),
            poll_interval,
            genesis,
        )
        .await
    }

    /// Run the poll loop.
    pub async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        match self.poll_forever(ctx).await {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }

    /// Poll the client forever in a loop or until canceled.
    async fn poll_forever(&mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        loop {
            self.poll_until_some(ctx).await?;
            ctx.sleep(self.poll_interval).await?;
        }
    }

    /// Poll the client until some data is returned and write it into the status.
    async fn poll_until_some(&mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        loop {
            match self.client.attestation_status(ctx).await {
                Ok(Some((genesis, next_batch_to_attest))) => {
                    self.status.update(genesis, next_batch_to_attest).await?;
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
struct LocalAttestationStatusClient {
    /// We don't expect the genesis to change while the main node is running,
    /// so we can just cache the genesis hash and return it for every request.
    genesis: attester::GenesisHash,
    batch_store: Arc<BatchStore>,
}

#[async_trait::async_trait]
impl AttestationStatusClient for LocalAttestationStatusClient {
    async fn attestation_status(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<(attester::GenesisHash, attester::BatchNumber)>> {
        let next_batch_to_attest = self.batch_store.next_batch_to_attest(ctx).await?;

        Ok(next_batch_to_attest.map(|n| (self.genesis, n)))
    }
}
