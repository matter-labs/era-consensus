//! Messages related to the consensus protocol.
use super::{BlockNumber, LeaderCommit, LeaderPrepare, Msg, ReplicaCommit, ReplicaPrepare};
use crate::validator;
use anyhow::Context;
use bit_vec::BitVec;
use std::{collections::BTreeMap, fmt};
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};

/// Current genesis encoding version
pub const CURRENT_GENESIS_ENCODING_VERSION: usize = 1;

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

/// Specification of a fork.
#[derive(Clone, Debug, PartialEq)]
pub struct Fork {
    /// Number of the fork.
    pub number: ForkNumber,
    /// First block of a fork.
    pub first_block: BlockNumber,
}

impl Default for Fork {
    fn default() -> Self {
        Self {
            number: ForkNumber(0),
            first_block: BlockNumber(0),
        }
    }
}

/// A struct that represents a set of validators. It is used to store the current validator set.
/// We represent each validator by its validator public key.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Committee {
    vec: Vec<WeightedValidator>,
    indexes: BTreeMap<validator::PublicKey, usize>,
    total_weight: u64,
}

impl Committee {
    /// Creates a new Committee from a list of validator public keys.
    pub fn new(validators: impl IntoIterator<Item = WeightedValidator>) -> anyhow::Result<Self> {
        let mut weighted_validators = BTreeMap::new();
        let mut total_weight: u64 = 0;
        for validator in validators {
            anyhow::ensure!(
                !weighted_validators.contains_key(&validator.key),
                "Duplicate validator in validator Committee"
            );
            anyhow::ensure!(
                validator.weight > 0,
                "Validator weight has to be a positive value"
            );
            total_weight = total_weight
                .checked_add(validator.weight)
                .context("Sum of weights overflows in validator Committee")?;
            weighted_validators.insert(validator.key.clone(), validator);
        }
        anyhow::ensure!(
            !weighted_validators.is_empty(),
            "Validator Committee must contain at least one validator"
        );
        Ok(Self {
            vec: weighted_validators.values().cloned().collect(),
            indexes: weighted_validators
                .values()
                .enumerate()
                .map(|(i, v)| (v.key.clone(), i))
                .collect(),
            total_weight,
        })
    }

    /// Iterates over weighted validators.
    pub fn iter(&self) -> impl Iterator<Item = &WeightedValidator> {
        self.vec.iter()
    }

    /// Iterates over validator keys.
    pub fn iter_keys(&self) -> impl Iterator<Item = &validator::PublicKey> {
        self.vec.iter().map(|v| &v.key)
    }

    /// Returns the number of validators.
    #[allow(clippy::len_without_is_empty)] // a valid `Committee` is always non-empty by construction
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    /// Returns true if the given validator is in the validator committee.
    pub fn contains(&self, validator: &validator::PublicKey) -> bool {
        self.indexes.contains_key(validator)
    }

    /// Get validator by its index in the committee.
    pub fn get(&self, index: usize) -> Option<&WeightedValidator> {
        self.vec.get(index)
    }

    /// Get the index of a validator in the committee.
    pub fn index(&self, validator: &validator::PublicKey) -> Option<usize> {
        self.indexes.get(validator).copied()
    }

    /// Computes the leader for the given view.
    pub fn view_leader(&self, view_number: ViewNumber) -> validator::PublicKey {
        let index = view_number.0 as usize % self.len();
        self.get(index).unwrap().key.clone()
    }

    /// Signature weight threshold for this validator committee.
    pub fn threshold(&self) -> u64 {
        threshold(self.total_weight())
    }

    /// Maximal weight of faulty replicas allowed in this validator committee.
    pub fn max_faulty_weight(&self) -> u64 {
        max_faulty_weight(self.total_weight())
    }

    /// Compute the sum of signers weights.
    /// Panics if signers length does not match the number of validators in committee
    pub fn weight(&self, signers: &Signers) -> u64 {
        assert_eq!(self.vec.len(), signers.len());
        self.vec
            .iter()
            .enumerate()
            .filter(|(i, _)| signers.0[*i])
            .map(|(_, v)| v.weight)
            .sum()
    }

    /// Sum of all validators' weight in the committee
    pub fn total_weight(&self) -> u64 {
        self.total_weight
    }
}

/// Calculate the consensus threshold, the minimum votes' weight for any consensus action to be valid,
/// for a given committee total weight.
pub fn threshold(total_weight: u64) -> u64 {
    total_weight - max_faulty_weight(total_weight)
}

/// Calculate the maximum allowed weight for faulty replicas, for a given total weight.
pub fn max_faulty_weight(total_weight: u64) -> u64 {
    // Calculate the allowed maximum weight of faulty replicas. We want the following relationship to hold:
    //      n = 5*f + 1
    // for n total weight and f faulty weight. This results in the following formula for the maximum
    // weight of faulty replicas:
    //      f = floor((n - 1) / 5)
    (total_weight - 1) / 5
}

/// Genesis of the blockchain, unique for each blockchain instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Genesis {
    // TODO(gprusak): add blockchain id here.
    /// Genesis encoding version
    pub encoding_version: usize,
    /// Set of validators of the chain.
    pub validators: Committee,
    /// Fork of the chain to follow.
    pub fork: Fork,
}

/// Hash of the genesis specification.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GenesisHash(pub(crate) Keccak256);

impl Genesis {
    /// Hash of the genesis.
    pub fn hash(&self) -> GenesisHash {
        GenesisHash(Keccak256::new(&zksync_protobuf::canonical(self)))
    }
}

impl Default for Genesis {
    fn default() -> Self {
        Self {
            encoding_version: CURRENT_GENESIS_ENCODING_VERSION,
            validators: Committee::default(),
            fork: Fork::default(),
        }
    }
}

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

/// Consensus messages.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsensusMsg {
    ReplicaPrepare(ReplicaPrepare),
    ReplicaCommit(ReplicaCommit),
    LeaderPrepare(LeaderPrepare),
    LeaderCommit(LeaderCommit),
}

impl ConsensusMsg {
    /// ConsensusMsg variant name.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ReplicaPrepare(_) => "ReplicaPrepare",
            Self::ReplicaCommit(_) => "ReplicaCommit",
            Self::LeaderPrepare(_) => "LeaderPrepare",
            Self::LeaderCommit(_) => "LeaderCommit",
        }
    }

    /// View of this message.
    pub fn view(&self) -> &View {
        match self {
            Self::ReplicaPrepare(m) => &m.view,
            Self::ReplicaCommit(m) => &m.view,
            Self::LeaderPrepare(m) => m.view(),
            Self::LeaderCommit(m) => m.view(),
        }
    }

    /// Protocol version of this message.
    pub fn protocol_version(&self) -> ProtocolVersion {
        self.view().protocol_version
    }
}

impl Variant<Msg> for ReplicaPrepare {
    fn insert(self) -> Msg {
        ConsensusMsg::ReplicaPrepare(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::ReplicaPrepare(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for ReplicaCommit {
    fn insert(self) -> Msg {
        ConsensusMsg::ReplicaCommit(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::ReplicaCommit(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for LeaderPrepare {
    fn insert(self) -> Msg {
        ConsensusMsg::LeaderPrepare(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::LeaderPrepare(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for LeaderCommit {
    fn insert(self) -> Msg {
        ConsensusMsg::LeaderCommit(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::LeaderCommit(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

/// View specification.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct View {
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// Fork this message belongs to.
    pub fork: ForkNumber,
    /// The number of the current view.
    pub number: ViewNumber,
}

impl View {
    /// Checks if `self` can occur after `b`.
    pub fn after(&self, b: &Self) -> bool {
        self.fork == b.fork && self.number > b.number && self.protocol_version >= b.protocol_version
    }
}

/// Struct that represents a bit map of validators. We use it to compactly store
/// which validators signed a given message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Signers(pub BitVec);

impl Signers {
    /// Constructs an empty signers set.
    pub fn new(n: usize) -> Self {
        Self(BitVec::from_elem(n, false))
    }

    /// Returns the number of signers, i.e. the number of validators that signed
    /// the particular message that this signer bitmap refers to.
    pub fn count(&self) -> usize {
        self.0.iter().filter(|b| *b).count()
    }

    /// Size of the corresponding validator Committee.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if there are no signers.
    pub fn is_empty(&self) -> bool {
        self.0.none()
    }
}

impl std::ops::BitOrAssign<&Self> for Signers {
    fn bitor_assign(&mut self, other: &Self) {
        self.0.or(&other.0);
    }
}

impl std::ops::BitAndAssign<&Self> for Signers {
    fn bitand_assign(&mut self, other: &Self) {
        self.0.and(&other.0);
    }
}

impl std::ops::BitAnd for &Signers {
    type Output = Signers;
    fn bitand(self, other: Self) -> Signers {
        let mut this = self.clone();
        this &= other;
        this
    }
}

/// A struct that represents a view number.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewNumber(pub u64);

impl ViewNumber {
    /// Get the next view number.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for ViewNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// An enum that represents the current phase of the consensus.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Prepare,
    Commit,
}

/// Validator representation inside a Committee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedValidator {
    /// Validator key
    pub key: validator::PublicKey,
    /// Validator weight inside the Committee.
    pub weight: u64,
}
