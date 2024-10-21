//! Messages related to the consensus protocol.
use super::{BlockNumber, LeaderSelectionMode, ViewNumber};
use crate::{attester, validator};
use std::{fmt, hash::Hash};
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};

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
    /// Set of validators of the chain.
    pub validators: validator::Committee,
    /// Set of attesters of the chain.
    pub attesters: Option<attester::Committee>,
    /// The mode used for selecting leader for a given view.
    pub leader_selection: LeaderSelectionMode,
}

impl GenesisRaw {
    /// Constructs Genesis with cached hash.
    pub fn with_hash(self) -> Genesis {
        let hash = GenesisHash(Keccak256::new(&zksync_protobuf::canonical(&self)));
        Genesis(self, hash)
    }
}

/// Hash of the genesis specification.
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

impl fmt::Debug for GenesisHash {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// Genesis with cached hash.
#[derive(Clone)]
pub struct Genesis(pub(crate) GenesisRaw, pub(crate) GenesisHash);

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

impl Genesis {
    /// Verifies correctness.
    pub fn verify(&self) -> anyhow::Result<()> {
        if let LeaderSelectionMode::Sticky(pk) = &self.leader_selection {
            if self.validators.index(pk).is_none() {
                anyhow::bail!("leader_selection sticky mode public key is not in committee");
            }
        } else if let LeaderSelectionMode::Rota(pks) = &self.leader_selection {
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

    /// Computes the leader for the given view.
    pub fn view_leader(&self, view: ViewNumber) -> validator::PublicKey {
        self.validators.view_leader(view, &self.leader_selection)
    }

    /// Hash of the genesis.
    pub fn hash(&self) -> GenesisHash {
        self.1
    }
}

/// Version of the consensus algorithm that the validator is using.
/// It allows to prevent misinterpretation of messages signed by validators
/// using different versions of the binaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub u32);

impl ProtocolVersion {
    /// 0 - development version; deprecated.
    /// 1 - development version
    pub const CURRENT: Self = Self(1);

    /// Returns the integer corresponding to this version.
    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Checks protocol version compatibility.
    pub fn compatible(&self, other: &ProtocolVersion) -> bool {
        // Currently using comparison.
        // This can be changed later to apply a minimum supported version.
        self.0 == other.0
    }
}

impl TryFrom<u32> for ProtocolVersion {
    type Error = anyhow::Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        // Currently, consensus doesn't define restrictions on the possible version. Unsupported
        // versions are filtered out on the BFT actor level instead.
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
