use std::fmt;

use zksync_consensus_crypto::keccak256::Keccak256;

/// A transaction propagated by the gossip network. Consensus layer does not interpret the transaction.
/// It is the responsibility of the execution layer to interpret the transaction.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Transaction(pub Vec<u8>);

impl Transaction {
    /// Hash of the transaction.
    pub fn hash(&self) -> Keccak256 {
        Keccak256::new(&self.0)
    }

    /// Returns the length of the transaction.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for Transaction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Transaction")
            .field("len", &self.0.len())
            .field("hash", &self.hash())
            .finish()
    }
}
