use std::fmt;

use anyhow::Context as _;
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};
use zksync_protobuf::ProtoFmt;

use super::v2;
use crate::{
    proto::validator as proto,
    validator::{GenesisHash, Msg},
};

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

/// A struct that represents an epoch number.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EpochNumber(pub u64);

impl EpochNumber {
    /// Get the next epoch number.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Get the previous epoch number.
    pub fn prev(self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

impl fmt::Display for EpochNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// Consensus messages.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsensusMsg {
    V2(v2::ChonkyMsg),
}

impl ConsensusMsg {
    /// ConsensusMsg variant name.
    pub fn label(&self) -> &'static str {
        match self {
            Self::V2(v2_msg) => match v2_msg {
                v2::ChonkyMsg::LeaderProposal(_) => "LeaderProposalV2",
                v2::ChonkyMsg::ReplicaCommit(_) => "ReplicaCommitV2",
                v2::ChonkyMsg::ReplicaTimeout(_) => "ReplicaTimeoutV2",
                v2::ChonkyMsg::ReplicaNewView(_) => "ReplicaNewViewV2",
            },
        }
    }

    /// View number of this message.
    pub fn view_number(&self) -> ViewNumber {
        match self {
            Self::V2(msg) => msg.view_number(),
        }
    }

    /// Hash of the genesis that defines the chain.
    pub fn genesis(&self) -> GenesisHash {
        match self {
            Self::V2(msg) => msg.genesis(),
        }
    }
}

impl Variant<Msg> for v2::ChonkyMsg {
    fn insert(self) -> Msg {
        ConsensusMsg::V2(self).insert()
    }
    #[allow(irrefutable_let_patterns)]
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::V2(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl ProtoFmt for ConsensusMsg {
    type Proto = proto::ConsensusMsg;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::consensus_msg::T;
        Ok(match r.t.as_ref().context("missing")? {
            T::V2(r) => Self::V2(ProtoFmt::read(r).context("v2")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::consensus_msg::T;

        let t = match self {
            Self::V2(x) => T::V2(x.build()),
        };

        Self::Proto { t: Some(t) }
    }
}
