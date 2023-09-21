//! Messages related to blocks.

use super::CommitQC;
use crypto::{sha256, ByteFmt, Text, TextFmt};
use std::fmt;

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
pub struct BlockHash(pub(crate) sha256::Sha256);

impl TextFmt for BlockHash {
    fn encode(&self) -> String {
        format!(
            "block_hash:sha256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("block_hash:sha256:")?.decode_hex().map(Self)
    }
}

impl fmt::Debug for BlockHash {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// A block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// Hash of the parent block.
    pub parent: BlockHash,
    /// Number of the block.
    pub number: BlockNumber,
    /// Payload of the block.
    pub payload: Vec<u8>,
}

impl Block {
    /// Returns the hash of the block.
    pub fn hash(&self) -> BlockHash {
        BlockHash(sha256::Sha256::new(&schema::canonical(self)))
    }

    /// Creates a genesis block.
    pub fn genesis(payload: Vec<u8>) -> Block {
        Block {
            parent: BlockHash(sha256::Sha256::default()),
            number: BlockNumber(0),
            payload,
        }
    }
}

/// A block that has been finalized by the consensus protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalBlock {
    /// The block.
    pub block: Block,
    /// Justification for the block. What guarantees that the block is final.
    pub justification: CommitQC,
}

impl FinalBlock {
    /// Creates a new finalized block.
    pub fn new(block: Block, justification: CommitQC) -> Self {
        assert_eq!(block.hash(), justification.message.proposal_block_hash);

        Self {
            block,
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
