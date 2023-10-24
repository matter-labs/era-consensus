//! Messages related to blocks.

use super::CommitQC;
use crypto::{sha256, ByteFmt, Text, TextFmt};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub(crate) u32);

impl ProtocolVersion {
    pub fn genesis() -> Self { Self(0) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload(pub Vec<u8>);

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PayloadHash(pub(crate) sha256::Sha256);

impl TextFmt for PayloadHash {
    fn encode(&self) -> String {
        format!(
            "payload:sha256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("payload:sha256:")?.decode_hex().map(Self)
    }
}

impl fmt::Debug for PayloadHash {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

impl Payload {
    pub fn hash(&self) -> PayloadHash {
        PayloadHash(sha256::Sha256::new(&self.0))
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
pub struct BlockHeaderHash(pub(crate) sha256::Sha256);

impl TextFmt for BlockHeaderHash {
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

impl fmt::Debug for BlockHeaderHash {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// A block header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockHeader {
    /// 
    pub protocol_version: ProtocolVersion,
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
        BlockHeaderHash(sha256::Sha256::new(&schema::canonical(self)))
    }

    /// Creates a genesis block.
    pub fn genesis(payload: PayloadHash) -> Self {
        Self {
            protocol_version: ProtocolVersion::genesis(),
            parent: BlockHeaderHash(sha256::Sha256::default()),
            number: BlockNumber(0),
            payload,
        }
    }

    pub fn new(parent: &BlockHeader, payload: PayloadHash) -> Self {
        Self {
            protocol_version: parent.protocol_version,
            parent: parent.hash(),
            number: parent.number.next(),
            payload,
        }
    }
}

/// A block that has been finalized by the consensus protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalBlock {
    pub header: BlockHeader,
    pub payload: Payload,
    /// Justification for the block. What guarantees that the block is final.
    pub justification: CommitQC,
}

impl FinalBlock {
    /// Creates a new finalized block.
    pub fn new(header: BlockHeader, payload :Payload, justification: CommitQC) -> Self {
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
