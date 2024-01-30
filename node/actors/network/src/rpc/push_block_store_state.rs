//! RPC for notifying peer about our BlockStore state.
use crate::{mux, proto::gossip as proto};
use anyhow::Context;
use zksync_concurrency::{limiter, time};
use zksync_consensus_storage::BlockStoreState;
use zksync_protobuf::{read_required, ProtoFmt};

/// PushBlockStoreState RPC.
#[derive(Debug)]
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 3;
    const INFLIGHT: u32 = 1;
    const RATE: limiter::Rate = limiter::Rate {
        burst: 2,
        refresh: time::Duration::milliseconds(500),
    };
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
            first: read_required(&message.first).context("first")?,
            last: read_required(&message.last).context("last")?,
        }))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            first: Some(self.0.first.build()),
            last: Some(self.0.last.build()),
        }
    }
}
