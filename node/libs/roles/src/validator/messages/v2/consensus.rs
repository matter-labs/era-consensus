use std::hash::Hash;

use anyhow::Context as _;
use bit_vec::BitVec;
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};
use zksync_protobuf::{read_required, required, ProtoFmt};

use super::{LeaderProposal, ReplicaCommit, ReplicaNewView, ReplicaTimeout};
use crate::{
    proto::validator as proto,
    validator::{EpochNumber, GenesisHash, Msg, Schedule, ViewNumber},
};

/// Consensus messages.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChonkyMsg {
    LeaderProposal(LeaderProposal),
    ReplicaCommit(ReplicaCommit),
    ReplicaNewView(ReplicaNewView),
    ReplicaTimeout(ReplicaTimeout),
}

impl ChonkyMsg {
    /// View number of this message.
    pub fn view_number(&self) -> ViewNumber {
        match self {
            Self::LeaderProposal(msg) => msg.view().number,
            Self::ReplicaCommit(msg) => msg.view.number,
            Self::ReplicaNewView(msg) => msg.view().number,
            Self::ReplicaTimeout(msg) => msg.view.number,
        }
    }

    /// Hash of the genesis that defines the chain.
    pub fn genesis(&self) -> GenesisHash {
        match self {
            Self::LeaderProposal(msg) => msg.view().genesis,
            Self::ReplicaCommit(msg) => msg.view.genesis,
            Self::ReplicaNewView(msg) => msg.view().genesis,
            Self::ReplicaTimeout(msg) => msg.view.genesis,
        }
    }
}

impl Variant<Msg> for LeaderProposal {
    fn insert(self) -> Msg {
        ChonkyMsg::LeaderProposal(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ChonkyMsg::LeaderProposal(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for ReplicaCommit {
    fn insert(self) -> Msg {
        ChonkyMsg::ReplicaCommit(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ChonkyMsg::ReplicaCommit(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for ReplicaNewView {
    fn insert(self) -> Msg {
        ChonkyMsg::ReplicaNewView(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ChonkyMsg::ReplicaNewView(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for ReplicaTimeout {
    fn insert(self) -> Msg {
        ChonkyMsg::ReplicaTimeout(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ChonkyMsg::ReplicaTimeout(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl ProtoFmt for ChonkyMsg {
    type Proto = proto::ChonkyMsgV2;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::chonky_msg_v2::T;
        Ok(match r.t.as_ref().context("missing")? {
            T::ReplicaCommit(r) => Self::ReplicaCommit(ProtoFmt::read(r).context("ReplicaCommit")?),
            T::ReplicaTimeout(r) => {
                Self::ReplicaTimeout(ProtoFmt::read(r).context("ReplicaTimeout")?)
            }
            T::ReplicaNewView(r) => {
                Self::ReplicaNewView(ProtoFmt::read(r).context("ReplicaNewView")?)
            }
            T::LeaderProposal(r) => {
                Self::LeaderProposal(ProtoFmt::read(r).context("LeaderProposal")?)
            }
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::chonky_msg_v2::T;

        let t = match self {
            Self::ReplicaCommit(x) => T::ReplicaCommit(x.build()),
            Self::ReplicaTimeout(x) => T::ReplicaTimeout(x.build()),
            Self::ReplicaNewView(x) => T::ReplicaNewView(x.build()),
            Self::LeaderProposal(x) => T::LeaderProposal(x.build()),
        };

        Self::Proto { t: Some(t) }
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
    /// The number of the current epoch.
    pub epoch: EpochNumber,
}

impl View {
    /// Verifies the view against the genesis.
    pub fn verify(&self, genesis: GenesisHash) -> anyhow::Result<()> {
        anyhow::ensure!(self.genesis == genesis, "genesis mismatch");
        Ok(())
    }

    /// Increments the view number.
    pub fn next_view(self) -> Self {
        Self {
            genesis: self.genesis,
            number: self.number.next(),
            epoch: self.epoch,
        }
    }

    /// Decrements the view number.
    pub fn prev_view(self) -> Option<Self> {
        self.number.prev().map(|number| Self {
            genesis: self.genesis,
            number,
            epoch: self.epoch,
        })
    }

    /// Increments the epoch number.
    pub fn next_epoch(&self) -> Self {
        Self {
            genesis: self.genesis,
            number: self.number,
            epoch: self.epoch.next(),
        }
    }

    /// Decrements the epoch number.
    pub fn prev_epoch(&self) -> Option<Self> {
        self.epoch.prev().map(|epoch| Self {
            genesis: self.genesis,
            number: self.number,
            epoch,
        })
    }
}

impl ProtoFmt for View {
    type Proto = proto::ViewV2;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            genesis: read_required(&r.genesis).context("genesis")?,
            number: ViewNumber(*required(&r.number).context("number")?),
            epoch: EpochNumber(*required(&r.epoch).context("epoch")?),
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            genesis: Some(self.genesis.build()),
            number: Some(self.number.0),
            epoch: Some(self.epoch.0),
        }
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

    /// Size of the corresponding Signer set.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if there are no signers.
    pub fn is_empty(&self) -> bool {
        self.0.none()
    }

    /// Compute the sum of signers weights.
    /// Panics if signers length does not match the number of validators in schedule.
    pub fn weight(&self, schedule: &Schedule) -> u64 {
        assert_eq!(self.len(), schedule.len());
        schedule
            .iter()
            .enumerate()
            .filter(|(i, _)| self.0[*i])
            .map(|(_, v)| v.weight)
            .sum()
    }
}

impl ProtoFmt for Signers {
    type Proto = zksync_protobuf::proto::std::BitVector;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ProtoFmt::read(r)?))
    }

    fn build(&self) -> Self::Proto {
        self.0.build()
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

impl ProtoFmt for Phase {
    type Proto = proto::PhaseV2;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::phase_v2::T;
        Ok(match required(&r.t)? {
            T::Prepare(_) => Self::Prepare,
            T::Commit(_) => Self::Commit,
            T::Timeout(_) => Self::Timeout,
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::phase_v2::T;
        let t = match self {
            Self::Prepare => T::Prepare(zksync_protobuf::proto::std::Void {}),
            Self::Commit => T::Commit(zksync_protobuf::proto::std::Void {}),
            Self::Timeout => T::Timeout(zksync_protobuf::proto::std::Void {}),
        };
        Self::Proto { t: Some(t) }
    }
}
