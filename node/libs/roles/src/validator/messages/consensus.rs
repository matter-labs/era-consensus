//! Messages related to the consensus protocol.
use super::{
    Genesis, GenesisHash, LeaderProposal, Msg, ReplicaCommit, ReplicaNewView, ReplicaTimeout,
};
use bit_vec::BitVec;
use std::{fmt, hash::Hash};
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

/// An enum that represents the current phase of the consensus.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Prepare,
    Commit,
}
