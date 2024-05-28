//! Defines storage layer for batches of blocks.
use zksync_consensus_roles::attester;

/// Trait for the shared state of batches between the consensus and the execution layer.
pub trait PersistentBatchStore {
    /// Get the L1 batch from storage with the highest number.
    fn last_batch(&self) -> attester::BatchNumber;
    /// Get the L1 batch QC from storage with the highest number.
    fn last_batch_qc(&self) -> attester::BatchNumber;
    /// Returns the batch with the given number.
    fn get_batch(&self, number: attester::BatchNumber) -> Option<attester::Batch>;
    /// Returns the QC of the batch with the given number.
    fn get_batch_qc(&self, number: attester::BatchNumber) -> Option<attester::BatchQC>;
    /// Store the given QC in the storage.
    fn store_qc(&self, qc: attester::BatchQC);
}
