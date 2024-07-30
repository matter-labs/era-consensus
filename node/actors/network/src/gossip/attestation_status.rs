use std::fmt;
use std::sync::Arc;

use zksync_concurrency::{ctx, sync, time};
use zksync_consensus_roles::attester;
use zksync_consensus_storage::BatchStore;

use crate::watch::Watch;

/// An interface which is used by attesters and nodes collecting votes over gossip to determine
/// which is the next batch they are all supposed to be voting on, according to the main node.
#[async_trait::async_trait]
pub trait AttestationStatusClient: 'static + fmt::Debug + Send + Sync {
    /// Get the next batch number for which the main node expects a batch QC to be formed.
    ///
    /// The API might return an error while genesis is being created, which we represent with `None`
    /// here and mean that we'll have to try again later.
    async fn next_batch_to_attest(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<attester::BatchNumber>>;
}

/// Coordinate the attestation by showing the status as seen by the main node.
#[derive(Debug, Clone)]
pub struct AttestationStatus {
    /// Next batch number where voting is expected.
    ///
    /// Its value is `None` until the background process polling the main node
    /// can establish a value to start from.
    pub next_batch_to_attest: Option<attester::BatchNumber>,
}

/// The subscription over the attestation status which voters can monitor for change.
pub type AttestationStatusReceiver = sync::watch::Receiver<AttestationStatus>;

/// A [Watch] over an [AttestationStatus] which we can use to notify components about
/// changes in the batch number the main node expects attesters to vote on.
pub struct AttestationStatusWatch(Watch<AttestationStatus>);

impl fmt::Debug for AttestationStatusWatch {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("AttestationStatusWatch")
            .finish_non_exhaustive()
    }
}

impl Default for AttestationStatusWatch {
    fn default() -> Self {
        Self(Watch::new(AttestationStatus {
            next_batch_to_attest: None,
        }))
    }
}

impl AttestationStatusWatch {
    /// Create a new [AttestationStatusWatch] paired up with an [AttestationStatusRunner] to keep it up to date.
    pub fn new(
        client: Box<dyn AttestationStatusClient>,
        poll_interval: time::Duration,
    ) -> (Arc<Self>, AttestationStatusRunner) {
        let status = Arc::new(AttestationStatusWatch::default());
        let runner = AttestationStatusRunner::new(status.clone(), client, poll_interval);
        (status, runner)
    }

    /// Subscribes to AttestationStatus updates.
    pub fn subscribe(&self) -> AttestationStatusReceiver {
        self.0.subscribe()
    }

    /// Set the next batch number to attest on and notify subscribers it changed.
    pub async fn update(&self, next_batch_to_attest: attester::BatchNumber) {
        let this = self.0.lock().await;
        this.send_if_modified(|status| {
            if status.next_batch_to_attest == Some(next_batch_to_attest) {
                return false;
            }
            status.next_batch_to_attest = Some(next_batch_to_attest);
            true
        });
    }
}

/// Use an [AttestationStatusClient] to periodically poll the main node and update the [AttestationStatusWatch].
pub struct AttestationStatusRunner {
    status: Arc<AttestationStatusWatch>,
    client: Box<dyn AttestationStatusClient>,
    poll_interval: time::Duration,
}

impl AttestationStatusRunner {
    /// Create a new runner to poll the main node.
    fn new(
        status: Arc<AttestationStatusWatch>,
        client: Box<dyn AttestationStatusClient>,
        poll_interval: time::Duration,
    ) -> Self {
        Self {
            status,
            client,
            poll_interval,
        }
    }

    /// Run the poll loop.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        loop {
            match self.client.next_batch_to_attest(ctx).await {
                Ok(Some(batch_number)) => {
                    self.status.update(batch_number).await;
                }
                Ok(None) => tracing::debug!("waiting for attestation status..."),
                Err(error) => tracing::error!(
                    ?error,
                    "failed to poll attestation status, retrying later..."
                ),
            }
            if let Err(ctx::Canceled) = ctx.sleep(self.poll_interval).await {
                return Ok(());
            }
        }
    }
}

/// Implement the attestation status for the main node by returning the next to vote on from the [BatchStore].
#[derive(Debug, Clone)]
pub struct LocalAttestationStatusClient(Arc<BatchStore>);

impl LocalAttestationStatusClient {
    /// Create local attestation client form a [BatchStore].
    pub fn new(store: Arc<BatchStore>) -> Self {
        Self(store)
    }
}

#[async_trait::async_trait]
impl AttestationStatusClient for LocalAttestationStatusClient {
    async fn next_batch_to_attest(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<attester::BatchNumber>> {
        self.0.next_batch_to_attest(ctx).await
    }
}
