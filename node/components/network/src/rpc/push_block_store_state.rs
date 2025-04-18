//! RPC for notifying peer about our BlockStore state.
use anyhow::Context;
use zksync_consensus_engine::{BlockStoreState, Last};
use zksync_consensus_roles::validator;
use zksync_protobuf::{read_optional_repr, read_required_repr, required, ProtoFmt, ProtoRepr};

use super::Capability;
use crate::proto::gossip as proto;

/// PushBlockStoreState RPC.
#[derive(Debug)]
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY: Capability = Capability::PushBlockStoreState;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "push_block_store_state";

    type Req = Req;
    type Resp = ();
}

/// Contains the freshest state of the sender's block store.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Req {
    // Block store state. Will be required once we drop
    // compatibility for `first` and `last` fields.
    pub state: BlockStoreState,
}

impl ProtoFmt for Req {
    type Proto = proto::PushBlockStoreState;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            state: read_required_repr(&r.state).context("state")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            state: Some(ProtoRepr::build(&self.state)),
        }
    }
}

impl ProtoRepr for proto::BlockStoreState {
    type Type = BlockStoreState;
    fn read(&self) -> anyhow::Result<Self::Type> {
        Ok(Self::Type {
            first: validator::BlockNumber(*required(&self.first).context("first")?),
            last: read_optional_repr(&self.last).context("last")?,
        })
    }
    fn build(this: &Self::Type) -> Self {
        Self {
            first: Some(this.first.0),
            last: this.last.as_ref().map(ProtoRepr::build),
        }
    }
}

impl ProtoRepr for proto::Last {
    type Type = Last;
    fn read(&self) -> anyhow::Result<Self::Type> {
        use proto::last::T;
        Ok(match self.t.as_ref().context("missing")? {
            T::PreGenesis(n) => Last::PreGenesis(validator::BlockNumber(*n)),
            T::Final(qc) => Last::FinalV1(ProtoFmt::read(qc).context("final")?),
            T::FinalV2(qc) => Last::FinalV2(ProtoFmt::read(qc).context("final_v2")?),
        })
    }
    fn build(this: &Self::Type) -> Self {
        use proto::last::T;
        Self {
            t: Some(match this {
                Last::PreGenesis(n) => T::PreGenesis(n.0),
                Last::FinalV1(qc) => T::Final(qc.build()),
                Last::FinalV2(qc) => T::FinalV2(qc.build()),
            }),
        }
    }
}
