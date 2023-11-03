//! Messages related to blocks.

use super::CommitQC;
use crypto::{keccak256, ByteFmt, Text, TextFmt};
use std::fmt;

/// Payload of the block. Consensus algorithm does not interpret the payload
/// (except for imposing a size limit for the payload). Proposing a payload
/// for a new block and interpreting the payload of the finalized blocks
/// should be implemented for the specific application of the consensus algorithm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload(pub Vec<u8>);

/// Hash of the Payload.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PayloadHash(pub(crate) keccak256::Keccak256);

impl TextFmt for PayloadHash {
    fn encode(&self) -> String {
        format!(
            "payload:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("payload:keccak256:")?.decode_hex().map(Self)
    }
}

impl fmt::Debug for PayloadHash {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

impl Payload {
    /// Hash of the payload.
    pub fn hash(&self) -> PayloadHash {
        PayloadHash(keccak256::Keccak256::new(&self.0))
    }
}

/// Sequential number of the block.
/// Genesis block has number 0.
/// For other blocks: block.number = block.parent.number + 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockNumber(pub u64);

impl BlockNumber {
    /// Returns the next block number.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Returns the previous block number.
    pub fn prev(self) -> Self {
        Self(self.0 - 1)
    }
}

impl fmt::Display for BlockNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// Hash of the block.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockHeaderHash(pub(crate) keccak256::Keccak256);

impl TextFmt for BlockHeaderHash {
    fn encode(&self) -> String {
        format!(
            "block_hash:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("block_hash:keccak256:")?.decode_hex().map(Self)
    }
}

impl fmt::Debug for BlockHeaderHash {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// A block header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockHeader {
    /// Hash of the parent block.
    pub parent: BlockHeaderHash,
    /// Number of the block.
    pub number: BlockNumber,
    /// Payload of the block.
    pub payload: PayloadHash,
}

impl BlockHeader {
    /// Returns the hash of the block.
    pub fn hash(&self) -> BlockHeaderHash {
        BlockHeaderHash(keccak256::Keccak256::new(&schema::canonical(self)))
    }

    /// Creates a genesis block.
    pub fn genesis(payload: PayloadHash) -> Self {
        Self {
            parent: BlockHeaderHash(keccak256::Keccak256::default()),
            number: BlockNumber(0),
            payload,
        }
    }

    /// Creates a child block for the given parent.
    pub fn new(parent: &BlockHeader, payload: PayloadHash) -> Self {
        Self {
            parent: parent.hash(),
            number: parent.number.next(),
            payload,
        }
    }
}

/// A block that has been finalized by the consensus protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalBlock {
    /// Header of the block.
    pub header: BlockHeader,
    /// Payload of the block. Should match `header.payload` hash.
    pub payload: Payload,
    /// Justification for the block. What guarantees that the block is final.
    pub justification: CommitQC,
}

impl FinalBlock {
    /// Creates a new finalized block.
    pub fn new(header: BlockHeader, payload: Payload, justification: CommitQC) -> Self {
        assert_eq!(header.payload, payload.hash());
        assert_eq!(header, justification.message.proposal);
        Self {
            header,
            payload,
            justification,
        }
    }
}

impl ByteFmt for FinalBlock {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ::schema::decode(bytes)
    }
    fn encode(&self) -> Vec<u8> {
        ::schema::encode(self)
    }
}

impl TextFmt for FinalBlock {
    fn encode(&self) -> String {
        format!("final_block:{}", hex::encode(ByteFmt::encode(self)))
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("final_block:")?.decode_hex()
    }
}
