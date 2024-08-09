//! Module to publish attestations over batches.
use anyhow::Context as _;
use std::sync::Arc;
use zksync_concurrency::{ctx, time};
pub use zksync_consensus_network::gossip::attestation::*;

/// An interface which is used by attesters and nodes collecting votes over gossip to determine
/// which is the next batch they are all supposed to be voting on, according to the main node.
///
/// This is a convenience interface to be used with the [AttestationStatusRunner].
#[async_trait::async_trait]
pub trait Client: 'static + Send + Sync {
    /// Get the next batch number for which the main node expects a batch QC to be formed.
    ///
    /// The API might return an error while genesis is being created, which we represent with `None`
    /// here and mean that we'll have to try again later.
    async fn config(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<Config>>;
}

/// Use an [Client] to periodically poll the main node and update the [AttestationStatusWatch].
///
/// This is provided for convenience.
pub struct Runner {
    /// state
    pub state: Arc<StateWatch>,
    /// client
    pub client: Box<dyn Client>,
    /// poll interval
    pub poll_interval: time::Duration,
}

impl Runner {
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
            match self.client.config(ctx).await {
                Ok(Some(config)) => {
                    self.state
                        .update_config(config)
                        .await
                        .context("update_config")?;
                }
                Ok(None) => tracing::debug!("waiting for attestation config..."),
                Err(error) => tracing::error!(
                    ?error,
                    "failed to poll attestation config, retrying later..."
                ),
            }
            if let Err(ctx::Canceled) = ctx.sleep(self.poll_interval).await {
                return Ok(());
            }
            ctx.sleep(self.poll_interval).await?;
        }
    }
}
