//! Messages related to the consensus protocol.
use super::{BlockNumber, LeaderCommit, LeaderPrepare, Msg, ReplicaCommit, ReplicaPrepare};
use crate::{attester, validator};
use anyhow::Context;
use bit_vec::BitVec;
use num_bigint::BigUint;
use std::{collections::BTreeMap, fmt, hash::Hash};
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};

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

/// The mode used for selecting leader for a given view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaderSelectionMode {
    /// Select in a round-robin fashion, based on validators' index within the set.
    RoundRobin,

    /// Select based on a sticky assignment to a specific validator.
    Sticky(validator::PublicKey),

    /// Select pseudo-randomly, based on validators' weights.
    Weighted,

    /// Select on a rotation of specific validator keys.
    Rota(Vec<validator::PublicKey>),
}

/// Calculates the pseudo-random eligibility of a leader based on the input and total weight.
pub(crate) fn leader_weighted_eligibility(input: u64, total_weight: u64) -> u64 {
    let input_bytes = input.to_be_bytes();
    let hash = Keccak256::new(&input_bytes);
    let hash_big = BigUint::from_bytes_be(hash.as_bytes());
    let total_weight_big = BigUint::from(total_weight);
    let ret_big = hash_big % total_weight_big;
    // Assumes that `ret_big` does not exceed 64 bits due to the modulo operation with a 64 bits-capped value.
    ret_big.to_u64_digits()[0]
}

/// A struct that represents a set of validators. It is used to store the current validator set.
/// We represent each validator by its validator public key.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Committee {
    vec: Vec<WeightedValidator>,
    indexes: BTreeMap<validator::PublicKey, usize>,
    total_weight: u64,
}

impl std::ops::Deref for Committee {
    type Target = Vec<WeightedValidator>;

    fn deref(&self) -> &Self::Target {
        &self.vec
    }
}

impl Committee {
    /// Creates a new Committee from a list of validator public keys.
    pub fn new(validators: impl IntoIterator<Item = WeightedValidator>) -> anyhow::Result<Self> {
        let mut map = BTreeMap::new();
        let mut total_weight: u64 = 0;
        for v in validators {
            anyhow::ensure!(
                !map.contains_key(&v.key),
                "Duplicate validator in validator Committee"
            );
            anyhow::ensure!(v.weight > 0, "Validator weight has to be a positive value");
            total_weight = total_weight
                .checked_add(v.weight)
                .context("Sum of weights overflows in validator Committee")?;
            map.insert(v.key.clone(), v);
        }
        anyhow::ensure!(
            !map.is_empty(),
            "Validator Committee must contain at least one validator"
        );
        let vec: Vec<_> = map.into_values().collect();
        Ok(Self {
            indexes: vec
                .iter()
                .enumerate()
                .map(|(i, v)| (v.key.clone(), i))
                .collect(),
            vec,
            total_weight,
        })
    }

    /// Iterates over validator keys.
    pub fn keys(&self) -> impl Iterator<Item = &validator::PublicKey> {
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
    pub fn view_leader(
        &self,
        view_number: ViewNumber,
        leader_selection: &LeaderSelectionMode,
    ) -> validator::PublicKey {
        match &leader_selection {
            LeaderSelectionMode::RoundRobin => {
                let index = view_number.0 as usize % self.len();
                self.get(index).unwrap().key.clone()
            }
            LeaderSelectionMode::Weighted => {
                let eligibility = leader_weighted_eligibility(view_number.0, self.total_weight);
                let mut offset = 0;
                for val in &self.vec {
                    offset += val.weight;
                    if eligibility < offset {
                        return val.key.clone();
                    }
                }
                unreachable!()
            }
            LeaderSelectionMode::Sticky(pk) => {
                let index = self.index(pk).unwrap();
                self.get(index).unwrap().key.clone()
            }
            LeaderSelectionMode::Rota(pks) => {
                let index = view_number.0 as usize % pks.len();
                let index = self.index(&pks[index]).unwrap();
                self.get(index).unwrap().key.clone()
            }
        }
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

/// Ethereum CHAIN_ID
/// `https://github.com/ethereum/EIPs/blob/master/EIPS/eip-155.md`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChainId(pub u64);

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
    /// Set of attesters of the chain.
    pub attesters: Option<attester::Committee>,
    /// The mode used for selecting leader for a given view.
    pub leader_selection: LeaderSelectionMode,
}

/// Hash of the genesis specification.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GenesisHash(pub(crate) Keccak256);

impl GenesisRaw {
    /// Constructs Genesis with cached hash.
    pub fn with_hash(self) -> Genesis {
        let hash = GenesisHash(Keccak256::new(&zksync_protobuf::canonical(&self)));
        Genesis(self, hash)
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

/// Genesis with cached hash.
#[derive(Clone)]
pub struct Genesis(GenesisRaw, GenesisHash);

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

    /// Hash of the genesis that defines the chain.
    pub fn genesis(&self) -> &GenesisHash {
        &self.view().genesis
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
    /// Genesis of the chain this view belongs to.
    pub genesis: GenesisHash,
    /// The number of the current view.
    pub number: ViewNumber,
}

impl View {
    /// Verifies the view against the genesis.
    pub fn verify(&self, genesis: &Genesis) -> anyhow::Result<()> {
        anyhow::ensure!(self.genesis == genesis.hash(), "genesis mismatch");
        Ok(())
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
