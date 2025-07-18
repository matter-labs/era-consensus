use std::{fmt, hash::Hash, ops::RangeInclusive};

use anyhow::Context as _;
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};
use zksync_protobuf::{read_optional, required, ProtoFmt};

use super::{BlockNumber, Schedule};
use crate::proto::validator::{self as proto};

/// Genesis of the blockchain, unique for each blockchain instance.
#[derive(Debug, Clone, PartialEq)]
pub struct GenesisRaw {
    /// ID of the blockchain.
    pub chain_id: ChainId,
    /// Number of the fork. Should be incremented every time the genesis is updated,
    /// i.e. whenever a hard fork is performed.
    pub fork_number: ForkNumber,
    /// Protocol version used by this fork.
    pub protocol_version: ProtocolVersion,
    /// First block of a fork.
    pub first_block: BlockNumber,
    /// The schedule of validators for the chain. If None, the chain is getting the
    /// validator schedule from the on-chain ConsensusRegistry contract.
    pub validators_schedule: Option<Schedule>,
}

impl GenesisRaw {
    /// Constructs Genesis with cached hash.
    pub fn with_hash(self) -> Genesis {
        let hash = GenesisHash(Keccak256::new(&zksync_protobuf::canonical(&self)));
        Genesis(self, hash)
    }
}

impl ProtoFmt for GenesisRaw {
    type Proto = proto::Genesis;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let protocol_version = ProtocolVersion(r.protocol_version.context("protocol_version")?);
        let validators_schedule = match protocol_version.0 {
            2 => read_optional(&r.validators_schedule).context("validators_schedule")?,
            _ => {
                unreachable!();
            }
        };

        Ok(GenesisRaw {
            chain_id: ChainId(*required(&r.chain_id).context("chain_id")?),
            fork_number: ForkNumber(*required(&r.fork_number).context("fork_number")?),
            first_block: BlockNumber(*required(&r.first_block).context("first_block")?),
            protocol_version,
            validators_schedule,
        })
    }

    fn build(&self) -> Self::Proto {
        let validators_schedule = match self.protocol_version.0 {
            2 => self.validators_schedule.as_ref().map(|x| x.build()),
            _ => {
                unreachable!();
            }
        };

        Self::Proto {
            chain_id: Some(self.chain_id.0),
            fork_number: Some(self.fork_number.0),
            first_block: Some(self.first_block.0),
            protocol_version: Some(self.protocol_version.0),
            validators_schedule,
        }
    }
}

/// Hash of the genesis specification.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GenesisHash(pub(crate) Keccak256);

impl TextFmt for GenesisHash {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("genesis_hash:keccak256:")?
            .decode_hex()
            .map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "genesis_hash:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl ProtoFmt for GenesisHash {
    type Proto = proto::GenesisHash;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.keccak256)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            keccak256: Some(self.0.encode()),
        }
    }
}

impl fmt::Debug for GenesisHash {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// Genesis with cached hash.
#[derive(Clone)]
pub struct Genesis(pub(crate) GenesisRaw, pub(crate) GenesisHash);

impl Genesis {
    /// Hash of the genesis.
    pub fn hash(&self) -> GenesisHash {
        self.1
    }
}

impl ProtoFmt for Genesis {
    type Proto = proto::Genesis;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(GenesisRaw::read(r)?.with_hash())
    }
    fn build(&self) -> Self::Proto {
        GenesisRaw::build(self)
    }
}

impl std::ops::Deref for Genesis {
    type Target = GenesisRaw;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq for Genesis {
    fn eq(&self, other: &Self) -> bool {
        self.1 == other.1
    }
}

impl fmt::Debug for Genesis {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(fmt)
    }
}

/// Version of the consensus algorithm that the validator is using.
/// It allows to prevent misinterpretation of messages signed by validators
/// using different versions of the binaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub u32);

impl ProtocolVersion {
    /// The latest production version of the protocol. Note that this might not be the version
    /// that the current chain is using. You also should not rely on this value to check compatibility,
    /// instead use the `compatible` method.
    pub const CURRENT: Self = Self(2);

    /// Returns the integer corresponding to this version.
    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Returns the range of supported protocol versions.
    pub fn supported_range() -> RangeInclusive<u32> {
        2..=2
    }

    /// Checks protocol version compatibility. Specifically, it checks which protocol
    /// versions are compatible with the current codebase. Old protocol versions can be
    /// deprecated, so a newer codebase might stop supporting an older protocol version even if
    /// no new protocol version is introduced.
    pub fn compatible(version: &ProtocolVersion) -> bool {
        version.0 == 2
    }
}

impl TryFrom<u32> for ProtocolVersion {
    type Error = anyhow::Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        // Currently, consensus doesn't define restrictions on the possible version. Unsupported
        // versions are filtered out on the BFT component level instead.
        Ok(Self(value))
    }
}

/// Number of the fork. Newer fork has higher number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForkNumber(pub u64);

impl ForkNumber {
    /// Next fork number.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// Ethereum CHAIN_ID
/// `https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChainId(pub u64);
