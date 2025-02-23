//! Messages related to the consensus protocol.
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};

use super::v1::{LeaderProposal, ReplicaCommit, ReplicaNewView, ReplicaTimeout, View};
use crate::validator::{GenesisHash, Msg};

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
