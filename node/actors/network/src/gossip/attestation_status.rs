use std::fmt;
use std::sync::Arc;
use zksync_concurrency::sync;
use zksync_consensus_roles::{attester};

use crate::watch::Watch;

/// Coordinate the attestation by showing the status as seen by the main node.
#[derive(Debug, Clone)]
pub struct AttestationStatus {
    pub batch_to_attest: attester::Batch,
    /// Committee for that batch.
    /// NOTE: the committee is not supposed to change often,
    /// so you might want to use `Arc<attester::Committee>` instead
    /// to avoid extra copying.
    pub committee: attester::Committee,
}

/// The subscription over the attestation status which voters can monitor for change.
pub type AttestationStatusReceiver = sync::watch::Receiver<Arc<AttestationStatus>>;

/// A [Watch] over an [AttestationStatus] which we can use to notify components about
/// changes in the batch number the main node expects attesters to vote on.
pub struct AttestationStatusWatch(Watch<Arc<AttestationStatus>>);

impl fmt::Debug for AttestationStatusWatch {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("AttestationStatusWatch")
            .finish_non_exhaustive()
    }
}

impl AttestationStatusWatch {
    /// Constructs AttestationStatusWatch.
    pub fn new(s :AttestationStatus) -> Self { Self(Watch::new(s)) }

    /// Subscribes to AttestationStatus updates.
    pub fn subscribe(&self) -> AttestationStatusReceiver {
        self.0.subscribe()
    }

    /// Set the next batch number to attest on and notify subscribers it changed.
    /// Returns an error if the update it not valid.
    pub async fn update(&self, new: AttestationStatus) -> anyhow::Result<()> {
        sync::try_send_modify(&self.0.lock().await, |s| {
            anyhow::ensure!(s.genesis == new.genesis, "tried to change genesis");
            anyhow::ensure!(s.batch_to_attest.number <= new.batch_to_attest.number, "tried to decrease batch number");
            if s.batch_to_attest.number == new.batch_to_attest.number {
                anyhow::ensure!(s == new, "tried to change attestation status for teh batch");
            }
            *s = Arc::new(new);
            Ok(())
        }).await
    }
}
