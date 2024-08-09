//! RPC for notifying peer about our BlockStore state.
use super::Capability;
use crate::proto::gossip as proto;
use anyhow::Context;
use zksync_consensus_roles::validator;
use zksync_consensus_storage::BlockStoreState;
use zksync_protobuf::{read_optional, required, ProtoFmt};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req(pub(crate) BlockStoreState);

impl ProtoFmt for Req {
    type Proto = proto::PushBlockStoreState;

    fn read(message: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(BlockStoreState {
            first: validator::BlockNumber(*required(&message.first).context("first")?),
            last: read_optional(&message.last).context("last")?,
        }))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            first: Some(self.0.first.0),
            last: self.0.last.as_ref().map(|x| x.build()),
        }
    }
}
