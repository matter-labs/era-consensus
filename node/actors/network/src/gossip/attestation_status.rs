use std::sync::Arc;

use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::attester;
use zksync_consensus_storage::BatchStore;

use crate::watch::Watch;

/// An interface which is used by attesters and nodes collecting votes over gossip to determine
/// which is the next batch they are all supposed to be voting on, according to the main node.
#[async_trait::async_trait]
pub trait AttestationStatusClient: 'static + std::fmt::Debug + Send + Sync {
    /// Get the next batch number for which the main node expects a batch QC to be formed.
    ///
    /// The API might return an error while genesis is being created, which we represent with `None`
    /// here and mean that we'll have to try again later.
    async fn next_batch_to_attest(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<attester::BatchNumber>>;
}

/// Implement the attestation status for the main node by returning the next to vote on from the [BatchStore].
#[derive(Debug, Clone)]
pub struct LocalAttestationStatus(Arc<BatchStore>);

impl LocalAttestationStatus {
    /// Create local attestation client form a [BatchStore].
    pub fn new(store: Arc<BatchStore>) -> Self {
        Self(store)
    }
}

#[async_trait::async_trait]
impl AttestationStatusClient for LocalAttestationStatus {
    async fn next_batch_to_attest(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<attester::BatchNumber>> {
        self.0.next_batch_to_attest(ctx).await
    }
}

/// Coordinate the attestation by showing the status as seen by the main node.
pub struct AttestationStatus {
    /// Next batch number where voting is expected.
    ///
    /// Its value is `None` until the background process polling the main node
    /// can establish a value to start from.
    pub next_batch_to_attest: Option<attester::BatchNumber>,
}

/// The subscription over the attestation status which votes can monitor for change.
pub type AttestationStatusReceiver = sync::watch::Receiver<AttestationStatus>;

/// A [Watch] over an [AttestationStatus] which we can use to notify components about
/// changes in the batch number the main node expects attesters to vote on.
pub(crate) struct AttestationStatusWatch(Watch<AttestationStatus>);

impl Default for AttestationStatusWatch {
    fn default() -> Self {
        Self(Watch::new(AttestationStatus {
            next_batch_to_attest: None,
        }))
    }
}

impl AttestationStatusWatch {
    /// Subscribes to AttestationStatus updates.
    pub(crate) fn subscribe(&self) -> AttestationStatusReceiver {
        self.0.subscribe()
    }

    /// Set the next batch number to attest on and notify subscribers it changed.
    pub(crate) async fn update(&self, next_batch_to_attest: attester::BatchNumber) {
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
