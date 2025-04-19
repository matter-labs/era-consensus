use std::fmt;

use anyhow::Context as _;
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};
use zksync_protobuf::{required, ProtoFmt};

use super::{v1, v2};
use crate::proto::validator as proto;

/// Represents a blockchain block across different consensus protocol versions (including pre-genesis blocks).
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Block with number `<genesis.first_block`.
    PreGenesis(PreGenesisBlock),
    /// Block with number `>=genesis.first_block`. For protocol version 1.
    FinalV1(v1::FinalBlock),
    /// Block with number `>=genesis.first_block`. For protocol version 1.
    FinalV2(v2::FinalBlock),
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

impl From<v2::FinalBlock> for Block {
    fn from(b: v2::FinalBlock) -> Self {
        Self::FinalV2(b)
    }
}

impl Block {
    /// Block number.
    pub fn number(&self) -> BlockNumber {
        match self {
            Self::PreGenesis(b) => b.number,
            Self::FinalV1(b) => b.number(),
            Self::FinalV2(b) => b.number(),
        }
    }

    /// Payload.
    pub fn payload(&self) -> &Payload {
        match self {
            Self::PreGenesis(b) => &b.payload,
            Self::FinalV1(b) => &b.payload,
            Self::FinalV2(b) => &b.payload,
        }
    }
}

impl ProtoFmt for Block {
    type Proto = proto::Block;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::block::T;
        Ok(match required(&r.t)? {
            T::FinalV2(b) => Block::FinalV2(ProtoFmt::read(b).context("block_v2")?),
            T::Final(b) => Block::FinalV1(ProtoFmt::read(b).context("block_v1")?),
            T::PreGenesis(b) => Block::PreGenesis(ProtoFmt::read(b).context("pre_genesis_block")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::block::T;
        Self::Proto {
            t: Some(match self {
                Block::FinalV2(b) => T::FinalV2(b.build()),
                Block::FinalV1(b) => T::Final(b.build()),
                Block::PreGenesis(b) => T::PreGenesis(b.build()),
            }),
        }
    }
}

/// A payload of a proposed block which is not known to be finalized yet.
/// Replicas have to persist such proposed payloads for liveness:
/// consensus may finalize a block without knowing a payload in case of reproposals.
/// Currently we do not store the BlockHeader, because it is always
/// available in the LeaderPrepare message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// Number of a block for which this payload has been proposed.
    pub number: BlockNumber,
    /// Proposed payload.
    pub payload: Payload,
}

impl ProtoFmt for Proposal {
    type Proto = proto::Proposal;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            number: BlockNumber(*required(&r.number).context("number")?),
            payload: Payload(required(&r.payload).context("payload")?.clone()),
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            number: Some(self.number.0),
            payload: Some(self.payload.0.clone()),
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

impl ProtoFmt for PayloadHash {
    type Proto = proto::PayloadHash;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.keccak256)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            keccak256: Some(self.0.encode()),
        }
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

impl ProtoFmt for PreGenesisBlock {
    type Proto = proto::PreGenesisBlock;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            number: BlockNumber(*required(&r.number).context("number")?),
            payload: Payload(required(&r.payload).context("payload")?.clone()),
            justification: Justification(
                required(&r.justification).context("justification")?.clone(),
            ),
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            number: Some(self.number.0),
            payload: Some(self.payload.0.clone()),
            justification: Some(self.justification.0.clone()),
        }
    }
}

/// Justification for pre-genesis blocks. Can be used in the future to prove inclusion of
/// a block in the base layer, so that we can sync pre-genesis blocks trustlessly.
#[derive(Debug, Clone, PartialEq)]
pub struct Justification(pub Vec<u8>);
