//! Messages related to blocks.
use std::fmt;

use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};

use super::v1;
/// Represents a blockchain block across different consensus protocol versions (including pre-genesis blocks).
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Block with number `<genesis.first_block`.
    PreGenesis(PreGenesisBlock),
    /// Block with number `>=genesis.first_block`. For protocol version 1.
    FinalV1(v1::FinalBlock),
}

impl From<PreGenesisBlock> for Block {
    fn from(b: PreGenesisBlock) -> Self {
        Self::PreGenesis(b)
    }
}

impl From<v1::FinalBlock> for Block {
    fn from(b: v1::FinalBlock) -> Self {
        Self::FinalV1(b)
    }
}

impl Block {
    /// Block number.
    pub fn number(&self) -> BlockNumber {
        match self {
            Self::PreGenesis(b) => b.number,
            Self::FinalV1(b) => b.number(),
        }
    }

    /// Payload.
    pub fn payload(&self) -> &Payload {
        match self {
            Self::PreGenesis(b) => &b.payload,
            Self::FinalV1(b) => &b.payload,
        }
    }
}

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

impl Payload {
    /// Hash of the payload.
    pub fn hash(&self) -> PayloadHash {
        PayloadHash(Keccak256::new(&self.0))
    }

    /// Returns the length of the payload.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the payload is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Hash of the Payload.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
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

/// Sequential number of the block.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
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

/// Block before `genesis.first_block`
/// with an external (non-consensus) justification.
#[derive(Debug, Clone, PartialEq)]
pub struct PreGenesisBlock {
    /// Block number.
    pub number: BlockNumber,
    /// Payload.
    pub payload: Payload,
    /// Justification.
    pub justification: Justification,
}

/// TODO: docs
#[derive(Debug, Clone, PartialEq)]
pub struct Justification(pub Vec<u8>);
