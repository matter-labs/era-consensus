use std::fmt;

use zksync_concurrency::sync;
use zksync_consensus_roles::attester;

use crate::watch::Watch;

/// Coordinate the attestation by showing the status as seen by the main node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationStatus {
    /// Next batch number where voting is expected.
    ///
    /// The node is expected to poll the main node during initialization until
    /// the batch to start from is established.
    pub next_batch_to_attest: attester::BatchNumber,
    /// The hash of the genesis of the chain to which the L1 batches belong.
    ///
    /// We don't expect to handle a regenesis on the fly without restarting the
    /// node, so this value is not expected to change; it's here only to stop
    /// any attempt at updating the status with a batch number that refers
    /// to a different fork.
    pub genesis: attester::GenesisHash,
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

impl AttestationStatusWatch {
    /// Create a new watch going from a specific batch number.
    pub fn new(
        genesis: attester::GenesisHash,
        next_batch_to_attest: attester::BatchNumber,
    ) -> Self {
        Self(Watch::new(AttestationStatus {
            genesis,
            next_batch_to_attest,
        }))
    }

    /// Subscribes to AttestationStatus updates.
    pub fn subscribe(&self) -> AttestationStatusReceiver {
        self.0.subscribe()
    }

    /// Set the next batch number to attest on and notify subscribers it changed.
    ///
    /// Fails if the genesis we want to update to is not the same as the watch was started with,
    /// because the rest of the system is not expected to be able to handle reorgs without a
    /// restart of the node.
    pub async fn update(
        &self,
        genesis: attester::GenesisHash,
        next_batch_to_attest: attester::BatchNumber,
    ) -> anyhow::Result<()> {
        let this = self.0.lock().await;
        {
            let status = this.borrow();
            anyhow::ensure!(
                status.genesis == genesis,
                "the attestation status genesis changed: {:?} -> {:?}",
                status.genesis,
                genesis
            );
        }
        this.send_if_modified(|status| {
            if status.next_batch_to_attest == next_batch_to_attest {
                return false;
            }
            status.next_batch_to_attest = next_batch_to_attest;
            true
        });
        Ok(())
    }
}
