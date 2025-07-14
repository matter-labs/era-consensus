use anyhow::Context as _;
use zksync_protobuf::{required, ProtoFmt};

use super::v2::ChonkyV2State;
use crate::proto::validator as proto;

/// The struct that contains the replica state to be persisted.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplicaState {
    V2(ChonkyV2State),
}

impl Default for ReplicaState {
    fn default() -> Self {
        Self::V2(ChonkyV2State::default())
    }
}

impl ProtoFmt for ReplicaState {
    type Proto = proto::ReplicaState;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::replica_state::T;
        Ok(match required(&r.t)? {
            T::V2(v2) => ReplicaState::V2(ProtoFmt::read(v2).context("v2")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::replica_state::T;
        Self::Proto {
            t: Some(match self {
                ReplicaState::V2(v2) => T::V2(v2.build()),
            }),
        }
    }
}
