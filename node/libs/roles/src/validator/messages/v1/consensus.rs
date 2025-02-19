//! Messages related to the consensus protocol.
use super::{LeaderProposal, ReplicaCommit, ReplicaNewView, ReplicaTimeout};
use crate::validator::{Committee, Genesis, GenesisHash, Msg, PublicKey};
use bit_vec::BitVec;
use num_bigint::BigUint;
use std::{fmt, hash::Hash};
use zksync_consensus_crypto::keccak256::Keccak256;
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};

/// Consensus messages.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsensusMsg {
    LeaderProposal(LeaderProposal),
    ReplicaCommit(ReplicaCommit),
    ReplicaNewView(ReplicaNewView),
    ReplicaTimeout(ReplicaTimeout),
}

impl ConsensusMsg {
    /// ConsensusMsg variant name.
    pub fn label(&self) -> &'static str {
        match self {
            Self::LeaderProposal(_) => "LeaderProposal",
            Self::ReplicaCommit(_) => "ReplicaCommit",
            Self::ReplicaNewView(_) => "ReplicaNewView",
            Self::ReplicaTimeout(_) => "ReplicaTimeout",
        }
    }

    /// View of this message.
    pub fn view(&self) -> View {
        match self {
            Self::LeaderProposal(msg) => msg.view(),
            Self::ReplicaCommit(msg) => msg.view,
            Self::ReplicaNewView(msg) => msg.view(),
            Self::ReplicaTimeout(msg) => msg.view,
        }
    }

    /// Hash of the genesis that defines the chain.
    pub fn genesis(&self) -> GenesisHash {
        self.view().genesis
    }
}

impl Variant<Msg> for LeaderProposal {
    fn insert(self) -> Msg {
        ConsensusMsg::LeaderProposal(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::LeaderProposal(this) = Variant::extract(msg)? else {
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

impl Variant<Msg> for ReplicaNewView {
    fn insert(self) -> Msg {
        ConsensusMsg::ReplicaNewView(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::ReplicaNewView(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for ReplicaTimeout {
    fn insert(self) -> Msg {
        ConsensusMsg::ReplicaTimeout(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::ReplicaTimeout(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

/// View specification.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    /// Increments the view number.
    pub fn next(self) -> Self {
        Self {
            genesis: self.genesis,
            number: ViewNumber(self.number.0 + 1),
        }
    }

    /// Decrements the view number.
    pub fn prev(self) -> Option<Self> {
        self.number.prev().map(|number| Self {
            genesis: self.genesis,
            number,
        })
    }
}

/// A struct that represents a view number.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewNumber(pub u64);

impl ViewNumber {
    /// Get the next view number.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Get the previous view number.
    pub fn prev(self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl fmt::Display for ViewNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// The mode used for selecting leader for a given view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaderSelectionMode {
    /// Select in a round-robin fashion, based on validators' index within the set.
    RoundRobin,
    /// Select based on a sticky assignment to a specific validator.
    Sticky(PublicKey),
    /// Select pseudo-randomly, based on validators' weights.
    Weighted,
    /// Select on a rotation of specific validator keys.
    Rota(Vec<PublicKey>),
}

impl LeaderSelectionMode {
    /// Computes the leader for the given view.
    pub fn view_leader(&self, view_number: ViewNumber, committee: &Committee) -> PublicKey {
        match &self {
            LeaderSelectionMode::RoundRobin => {
                let index = view_number.0 as usize % committee.len();
                committee.get(index).unwrap().key.clone()
            }
            LeaderSelectionMode::Weighted => {
                let eligibility =
                    Self::leader_weighted_eligibility(view_number.0, committee.total_weight());
                let mut offset = 0;
                for val in committee.iter() {
                    offset += val.weight;
                    if eligibility < offset {
                        return val.key.clone();
                    }
                }
                unreachable!()
            }
            LeaderSelectionMode::Sticky(pk) => {
                let index = committee.index(pk).unwrap();
                committee.get(index).unwrap().key.clone()
            }
            LeaderSelectionMode::Rota(pks) => {
                let index = view_number.0 as usize % pks.len();
                let index = committee.index(&pks[index]).unwrap();
                committee.get(index).unwrap().key.clone()
            }
        }
    }

    /// Calculates the pseudo-random eligibility of a leader based on the input and total weight.
    pub fn leader_weighted_eligibility(input: u64, total_weight: u64) -> u64 {
        let input_bytes = input.to_be_bytes();
        let hash = Keccak256::new(&input_bytes);
        let hash_big = BigUint::from_bytes_be(hash.as_bytes());
        let total_weight_big = BigUint::from(total_weight);
        let ret_big = hash_big % total_weight_big;
        // Assumes that `ret_big` does not exceed 64 bits due to the modulo operation with a 64 bits-capped value.
        ret_big.to_u64_digits()[0]
    }
}

/// Struct that represents a bit map of validators. We use it to compactly store
/// which validators signed a given message.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Signers(pub BitVec);

impl Signers {
    /// Constructs a new Signers bitmap with the given number of validators. All
    /// bits are set to false.
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

    /// Compute the sum of signers weights.
    /// Panics if signers length does not match the number of validators in committee
    pub fn weight(&self, committee: &Committee) -> u64 {
        assert_eq!(self.len(), committee.len());
        committee
            .iter()
            .enumerate()
            .filter(|(i, _)| self.0[*i])
            .map(|(_, v)| v.weight)
            .sum()
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

/// An enum that represents the current phase of the consensus.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Prepare,
    Commit,
    Timeout,
}
