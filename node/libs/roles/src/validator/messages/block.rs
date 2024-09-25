//! Messages related to blocks.

use super::{CommitQC, CommitQCVerifyError};
use std::fmt;
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};

/// Payload of the block. Consensus algorithm does not interpret the payload
/// (except for imposing a size limit for the payload). Proposing a payload
/// for a new block and interpreting the payload of the finalized blocks
/// should be implemented for the specific application of the consensus algorithm.
#[derive(Clone, PartialEq, Eq)]
pub struct Payload(pub Vec<u8>);

impl fmt::Debug for Payload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Payload")
            .field("len", &self.0.len())
            .field("hash", &self.hash())
            .finish()
    }
}

/// Hash of the Payload.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PayloadHash(pub(crate) Keccak256);

impl TextFmt for PayloadHash {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("payload:keccak256:")?.decode_hex().map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "payload:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl fmt::Debug for PayloadHash {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

impl Payload {
    /// Hash of the payload.
    pub fn hash(&self) -> PayloadHash {
        PayloadHash(Keccak256::new(&self.0))
    }
}

/// Sequential number of the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockNumber(pub u64);

impl BlockNumber {
    /// Returns the next block number.
    pub fn next(self) -> Self {
        Self(self.0.checked_add(1).unwrap())
    }

    /// Returns the previous block number.
    pub fn prev(self) -> Option<Self> {
        Some(Self(self.0.checked_sub(1)?))
    }
}

impl std::ops::Add<u64> for BlockNumber {
    type Output = BlockNumber;
    fn add(self, n: u64) -> Self {
        Self(self.0.checked_add(n).unwrap())
    }
}

impl fmt::Display for BlockNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// A block header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockHeader {
    /// Number of the block.
    pub number: BlockNumber,
    /// Payload of the block.
    pub payload: PayloadHash,
}

/// A block that has been finalized by the consensus protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalBlock {
    /// Payload of the block. Should match `header.payload` hash.
    pub payload: Payload,
    /// Justification for the block. What guarantees that the block is final.
    pub justification: CommitQC,
}

impl FinalBlock {
    /// Creates a new finalized block.
    pub fn new(payload: Payload, justification: CommitQC) -> Self {
        assert_eq!(justification.header().payload, payload.hash());
        Self {
            payload,
            justification,
        }
    }

    /// Header of the block.
    pub fn header(&self) -> &BlockHeader {
        &self.justification.message.proposal
    }

    /// Number of the block.
    pub fn number(&self) -> BlockNumber {
        self.header().number
    }

    /// Verifies internal consistency of this block.
    pub fn verify(&self, genesis: &super::Genesis) -> Result<(), BlockValidationError> {
        let payload_hash = self.payload.hash();
        if payload_hash != self.header().payload {
            return Err(BlockValidationError::HashMismatch {
                header_hash: self.header().payload,
                payload_hash,
            });
        }
        self.justification
            .verify(genesis)
            .map_err(BlockValidationError::Justification)
    }
}

impl ByteFmt for FinalBlock {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        zksync_protobuf::decode(bytes)
    }

    fn encode(&self) -> Vec<u8> {
        zksync_protobuf::encode(self)
    }
}

/// Errors that can occur validating a `FinalBlock` received from a node.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BlockValidationError {
    /// Block payload doesn't match the block header.
    #[error(
        "block payload doesn't match the block header (hash in header: {header_hash:?}, \
             payload hash: {payload_hash:?})"
    )]
    HashMismatch {
        /// Payload hash in block header.
        header_hash: PayloadHash,
        /// Hash of the payload.
        payload_hash: PayloadHash,
    },
    /// Failed verifying quorum certificate.
    #[error("failed verifying quorum certificate: {0:#?}")]
    Justification(#[source] CommitQCVerifyError),
}


/// TODO: docs
#[derive(Debug,Clone,PartialEq)]
pub struct Justification(pub Vec<u8>);

/// Block before `genesis.first_block`
/// with an external (non-consensus) justification.
#[derive(Debug,Clone,PartialEq)]
pub struct PreGenesisBlock {
    /// Block number.
    pub number: BlockNumber,
    /// Payload.
    pub payload: Payload,
    /// Justification.
    pub justification: Justification,
}

/// TODO: docs
#[derive(Debug,Clone,PartialEq)]
pub enum Block {
    /// Block with number `<genesis.first_block`.
    PreGenesis(PreGenesisBlock),
    /// Block with number `>=genesis.first_block`.
    Final(FinalBlock),
}

impl From<PreGenesisBlock> for Block {
    fn from(b: PreGenesisBlock) -> Self { Self::PreGenesis(b) }
}

impl From<FinalBlock> for Block {
    fn from(b: FinalBlock) -> Self { Self::Final(b) }
}

impl Block {
    /// Block number.
    pub fn number(&self) -> BlockNumber {
        match self {
            Self::PreGenesis(b) => b.number,
            Self::Final(b) => b.number(),
        }
    } 
}


