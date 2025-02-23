//! Messages related to the consensus protocol.
use super::{v1, BlockNumber, Committee};
use crate::validator;
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
    pub validators: Committee,
    /// The mode used for selecting leader for a given view.
    pub leader_selection: v1::LeaderSelectionMode,
}

impl GenesisRaw {
    /// Constructs Genesis with cached hash.
    pub fn with_hash(self) -> Genesis {
        let hash = GenesisHash(Keccak256::new(&zksync_protobuf::canonical(&self)));
        Genesis(self, hash)
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

    /// Computes the leader for the given view.
    pub fn view_leader(&self, view: v1::ViewNumber) -> validator::PublicKey {
        self.leader_selection.view_leader(view, &self.validators)
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
        version.0 == 1
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
