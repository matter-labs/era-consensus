use std::hash::Hash;

use anyhow::Context as _;
use bit_vec::BitVec;
use zksync_protobuf::{read_required, required, ProtoFmt};

use super::Committee;
use crate::{
    proto::validator as proto,
    validator::{GenesisHash, ViewNumber},
};

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
    pub fn verify(&self, genesis_hash: GenesisHash) -> anyhow::Result<()> {
        anyhow::ensure!(self.genesis == genesis_hash, "genesis mismatch");
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

impl ProtoFmt for View {
    type Proto = proto::View;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            genesis: read_required(&r.genesis).context("genesis")?,
            number: ViewNumber(*required(&r.number).context("number")?),
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            genesis: Some(self.genesis.build()),
            number: Some(self.number.0),
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
    type Proto = proto::Phase;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::phase::T;
        Ok(match required(&r.t)? {
            T::Prepare(_) => Self::Prepare,
            T::Commit(_) => Self::Commit,
            T::Timeout(_) => Self::Timeout,
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::phase::T;
        let t = match self {
            Self::Prepare => T::Prepare(zksync_protobuf::proto::std::Void {}),
            Self::Commit => T::Commit(zksync_protobuf::proto::std::Void {}),
            Self::Timeout => T::Timeout(zksync_protobuf::proto::std::Void {}),
        };
        Self::Proto { t: Some(t) }
    }
}
