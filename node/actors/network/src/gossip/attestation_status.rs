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
    // The hash of the genesis of the chain to which the L1 batches belong.
    //
    // A change in this value would indicate a reorg on the main node.
    // On the main node itself this is not expected to change because
    // a reorg involves a restart, and a regenesis happens before the
    // executor is started. If it could happen, it could be used to
    // signal to the `BatchVotes` that votes need to be cleared and
    // potentially discarded votes received over gossip would need
    // to be re-acquired (which doesn't happen at the moment unless
    // the connection is re-established).
    //
    // It is not added yet as the system is not expected to be able
    // to handle changes in the value.
    //pub genesis: attester::GenesisHash,
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
    pub fn new(next_batch_to_attest: attester::BatchNumber) -> Self {
        Self(Watch::new(AttestationStatus {
            next_batch_to_attest,
        }))
    }

    /// Subscribes to AttestationStatus updates.
    pub fn subscribe(&self) -> AttestationStatusReceiver {
        self.0.subscribe()
    }

    /// Set the next batch number to attest on and notify subscribers it changed.
    pub async fn update(&self, next_batch_to_attest: attester::BatchNumber) {
        let this = self.0.lock().await;
        this.send_if_modified(|status| {
            if status.next_batch_to_attest == next_batch_to_attest {
                return false;
            }
            status.next_batch_to_attest = next_batch_to_attest;
            true
        });
    }
}
