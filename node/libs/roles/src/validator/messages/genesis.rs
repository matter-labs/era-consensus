use std::{fmt, hash::Hash};

use anyhow::Context as _;
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};
use zksync_protobuf::{read_required, required, ProtoFmt};

use super::{v1, BlockNumber};
use crate::proto::validator as proto;

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
    /// Set of validators of the chain. Only valid for protocol version 1.
    pub validators: v1::Committee,
    /// The mode used for selecting leader for a given view. Only valid for protocol version 1.
    pub leader_selection: v1::LeaderSelectionMode,
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
        let validators: Vec<_> = r
            .validators_v1
            .iter()
            .enumerate()
            .map(|(i, v)| v1::WeightedValidator::read(v).context(i))
            .collect::<Result<_, _>>()
            .context("validators_v1")?;
        Ok(GenesisRaw {
            chain_id: ChainId(*required(&r.chain_id).context("chain_id")?),
            fork_number: ForkNumber(*required(&r.fork_number).context("fork_number")?),
            first_block: BlockNumber(*required(&r.first_block).context("first_block")?),

            protocol_version: ProtocolVersion(r.protocol_version.context("protocol_version")?),
            validators: v1::Committee::new(validators.into_iter()).context("validators_v1")?,
            leader_selection: read_required(&r.leader_selection).context("leader_selection")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            chain_id: Some(self.chain_id.0),
            fork_number: Some(self.fork_number.0),
            first_block: Some(self.first_block.0),

            protocol_version: Some(self.protocol_version.0),
            validators_v1: self.validators.iter().map(|v| v.build()).collect(),
            leader_selection: Some(self.leader_selection.build()),
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
    /// Verifies correctness.
    pub fn verify(&self) -> anyhow::Result<()> {
        if let v1::LeaderSelectionMode::Sticky(pk) = &self.leader_selection {
            if self.validators.index(pk).is_none() {
                anyhow::bail!("leader_selection sticky mode public key is not in committee");
            }
        } else if let v1::LeaderSelectionMode::Rota(pks) = &self.leader_selection {
            for pk in pks {
                if self.validators.index(pk).is_none() {
                    anyhow::bail!(
                        "leader_selection rota mode public key is not in committee: {pk:?}"
                    );
                }
            }
        }

        Ok(())
    }

    /// Hash of the genesis.
    pub fn hash(&self) -> GenesisHash {
        self.1
    }
}

impl ProtoFmt for Genesis {
    type Proto = proto::Genesis;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let genesis = GenesisRaw::read(r)?.with_hash();
        genesis.verify()?;
        Ok(genesis)
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
    pub const CURRENT: Self = Self(1);

    /// Returns the integer corresponding to this version.
    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Checks protocol version compatibility. Specifically, it checks which protocol
    /// versions are compatible with the current codebase. Old protocol versions can be
    /// deprecated, so a newer codebase might stop supporting an older protocol version even if
    /// no new protocol version is introduced.
    pub fn compatible(version: &ProtocolVersion) -> bool {
        version.0 == 1 || version.0 == 2
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
