use std::fmt;

use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};

/// A transaction propagated by the gossip network. Consensus layer does not interpret the transaction.
/// It is the responsibility of the application layer to interpret the transaction.
#[derive(Clone, PartialEq, Eq)]
pub struct Transaction(pub Vec<u8>);

impl Transaction {
    /// Hash of the transaction.
    pub fn hash(&self) -> TxHash {
        TxHash(Keccak256::new(&self.0))
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

/// The hash of a transaction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TxHash(pub(crate) Keccak256);

impl TextFmt for TxHash {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("tx_hash:keccak256:")?.decode_hex().map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "tx_hash:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl fmt::Debug for TxHash {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}
