//! RPC for fetching a batch from peer.
use crate::{mux, proto::gossip as proto};
use anyhow::Context as _;
use zksync_consensus_roles::attester;
use zksync_consensus_storage::BatchStoreState;
use zksync_protobuf::{read_optional, required, ProtoFmt};

/// PushBatchStoreState RPC.
#[derive(Debug)]
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 7;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "push_batch_store_state";

    type Req = Req;
    type Resp = ();
}

/// Contains the freshest state of the sender's batch store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req(pub(crate) BatchStoreState);

impl ProtoFmt for Req {
    type Proto = proto::PushBatchStoreState;

    fn read(message: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(BatchStoreState {
            first: attester::BatchNumber(*required(&message.first).context("first")?),
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
