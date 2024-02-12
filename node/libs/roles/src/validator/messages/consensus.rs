//! Messages related to the consensus protocol.
use super::*;
use crate::{validator};
use bit_vec::BitVec;
use std::collections::{BTreeMap, BTreeSet};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};
use zksync_consensus_crypto::{TextFmt, ByteFmt, Text, keccak256::Keccak256};
use std::fmt;

/// Version of the consensus algorithm that the validator is using.
/// It allows to prevent misinterpretation of messages signed by validators
/// using different versions of the binaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub u32);

impl ProtocolVersion {
    /// Earliest protocol version.
    pub const EARLIEST: Self = Self(0);

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

/// Identifier of the fork that this consensus instance is building blocks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForkId(pub usize);

/// Specification of a fork.
#[derive(Clone,Debug, PartialEq)]
pub struct Fork {
    /// First block of a fork.
    pub first_block: BlockNumber,
    /// Parent fo the first block of a fork.
    pub first_parent: Option<BlockHeaderHash>,
}

impl Default for Fork {
    fn default() -> Self { Self { first_block: BlockNumber(0), first_parent: None } }
}

/// History of forks of the blockchain.
/// It allows to determine which which fork contains block with the given number.
/// Current fork is the one with the highest fork id.
#[derive(Debug,Clone,PartialEq)]
pub struct ForkSet(pub(in crate::validator) Vec<Fork>);

impl ForkSet {
    /// Constructs a new fork set.
    pub fn new(fork:Fork) -> Self { Self(vec![fork]) }
}

impl ForkSet {
    /// Inserts a new fork to the fork set.
    pub fn push(&mut self, fork: Fork) {
        self.0.push(fork);
    }

    /// Current fork that node is participating in. 
    pub fn current(&self) -> ForkId { ForkId(self.0.len()-1) }
    /// First block of the current fork.
    pub fn first_block(&self) -> BlockNumber { self.0.last().unwrap().first_block }
    /// Parent of the first block of the current fork.
    pub fn first_parent(&self) -> Option<BlockHeaderHash> { self.0.last().unwrap().first_parent }

    /// Finds the fork which the given block belongs to.
    /// This should be used to verify the quorum certificate
    /// on a finalized block fetched from the network.
    pub fn find(&self, block: BlockNumber) -> Option<ForkId> {
        for i in (0..self.0.len()).rev() {
            if self.0[i].first_block <= block {
                return Some(ForkId(i));
            }
        }
        None
    }
}

/// A struct that represents a set of validators. It is used to store the current validator set.
/// We represent each validator by its validator public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatorSet {
    vec: Vec<validator::PublicKey>,
    map: BTreeMap<validator::PublicKey, usize>,
}

impl ValidatorSet {
    /// Creates a new ValidatorSet from a list of validator public keys.
    pub fn new(validators: impl IntoIterator<Item = validator::PublicKey>) -> anyhow::Result<Self> {
        let mut set = BTreeSet::new();
        for validator in validators {
            anyhow::ensure!(set.insert(validator),"Duplicate validator in ValidatorSet");
        }
        anyhow::ensure!(!set.is_empty(), "ValidatorSet must contain at least one validator");
        Ok(Self {
            vec: set.iter().cloned().collect(),
            map: set.into_iter().enumerate().map(|(i, pk)| (pk, i)).collect(),
        })
    }

    /// Iterates over validators.
    pub fn iter(&self) -> impl Iterator<Item = &validator::PublicKey> {
        self.vec.iter()
    }

    /// Returns the number of validators.
    #[allow(clippy::len_without_is_empty)] // a valid `ValidatorSet` is always non-empty by construction
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    /// Returns true if the given validator is in the validator set.
    pub fn contains(&self, validator: &validator::PublicKey) -> bool {
        self.map.contains_key(validator)
    }

    /// Get validator by its index in the set.
    pub fn get(&self, index: usize) -> Option<&validator::PublicKey> {
        self.vec.get(index)
    }

    /// Get the index of a validator in the set.
    pub fn index(&self, validator: &validator::PublicKey) -> Option<usize> {
        self.map.get(validator).copied()
    }

    /// Computes the validator for the given view.
    pub fn view_leader(&self, view_number: ViewNumber) -> validator::PublicKey {
        let index = view_number.0 as usize % self.len();
        self.get(index).unwrap().clone()
    }
   
    /// Signature threshold for this validator set.
    pub fn threshold(&self) -> usize {
        threshold(self.len())
    }
   
    /// Maximal number of faulty replicas allowed in this validator set.
    pub fn faulty_replicas(&self) -> usize {
        faulty_replicas(self.len())
    }
}

/// Calculate the consensus threshold, the minimum number of votes for any consensus action to be valid,
/// for a given number of replicas.
pub fn threshold(n: usize) -> usize {
    n-faulty_replicas(n)
}

/// Calculate the maximum number of faulty replicas, for a given number of replicas.
pub fn faulty_replicas(n: usize) -> usize {
    // Calculate the allowed maximum number of faulty replicas. We want the following relationship to hold:
    //      n = 5*f + 1
    // for n total replicas and f faulty replicas. This results in the following formula for the maximum
    // number of faulty replicas:
    //      f = floor((n - 1) / 5)
    // Because of this, it doesn't make sense to have 5*f + 2 or 5*f + 3 replicas. It won't increase the number
    // of allowed faulty replicas.
    (n - 1) / 5
}



/// Genesis of the blockchain, unique for each blockchain instance.
#[derive(Debug,Clone, PartialEq)]
pub struct Genesis {
    // TODO(gprusak): add blockchain id here.
    /// Set of validators of the chain.
    pub validators: ValidatorSet,
    /// Forks history, `forks.current()` indicates the current fork.
    pub forks: ForkSet,
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
            Self::LeaderPrepare(m) => &m.view(),
            Self::LeaderCommit(m) => &m.view(),
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
#[derive(Clone,Debug,PartialEq,Eq, PartialOrd, Ord, Hash)]
pub struct View {
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// Fork this message belongs to.
    pub fork: ForkId,
    /// The number of the current view.
    pub number: ViewNumber,
}

impl View {
    /// Checks if `self` can occur after `b`.
    pub fn after(&self, b: &Self) -> bool {
        self.fork == b.fork &&
        self.number > b.number &&
        self.protocol_version >= b.protocol_version
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


    /// Size of the corresponding ValidatorSet.
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

    /// Get the previous view number.
    pub fn prev(self) -> Self {
        Self(self.0 - 1)
    }
}

/// An enum that represents the current phase of the consensus.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Prepare,
    Commit,
}
