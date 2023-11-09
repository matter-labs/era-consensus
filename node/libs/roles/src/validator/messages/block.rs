//! Messages related to blocks.

use super::CommitQC;
use std::fmt;
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};

/// Payload of the block. Consensus algorithm does not interpret the payload
/// (except for imposing a size limit for the payload). Proposing a payload
/// for a new block and interpreting the payload of the finalized blocks
/// should be implemented for the specific application of the consensus algorithm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload(pub Vec<u8>);

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

/// Hash of the block header.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockHeaderHash(pub(crate) Keccak256);

impl BlockHeaderHash {
    /// Interprets the specified `bytes` as a block header hash digest (i.e., a reverse operation to [`Self::as_bytes()`]).
    /// It is caller's responsibility to ensure that `bytes` are actually a block header hash digest.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Keccak256::from_bytes(bytes))
    }

    /// Returns a reference to the bytes of this hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }
}

impl TextFmt for BlockHeaderHash {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("block_header_hash:keccak256:")?
            .decode_hex()
            .map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "block_header_hash:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl fmt::Debug for BlockHeaderHash {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
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
        BlockHeaderHash(Keccak256::new(&zksync_protobuf::canonical(self)))
    }

    /// Creates a genesis block.
    pub fn genesis(payload: PayloadHash) -> Self {
        Self {
            parent: BlockHeaderHash(Keccak256::default()),
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
        zksync_protobuf::decode(bytes)
    }

    fn encode(&self) -> Vec<u8> {
        zksync_protobuf::encode(self)
    }
}

impl TextFmt for FinalBlock {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("final_block:")?.decode_hex()
    }

    fn encode(&self) -> String {
        format!("final_block:{}", hex::encode(ByteFmt::encode(self)))
    }
}
