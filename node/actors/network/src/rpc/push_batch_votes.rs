//! Defines RPC for passing consensus messages.
use crate::{mux, proto::gossip as proto};
use anyhow::Context as _;
use std::sync::Arc;
use zksync_consensus_roles::attester::{self, Batch};
use zksync_protobuf::ProtoFmt;

/// PushBatchVotes RPC.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 5;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "push_batch_votes";
    type Req = Req;
    type Resp = ();
}

/// Signed batch message that the receiving peer should process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req(pub(crate) Vec<Arc<attester::Signed<Batch>>>);

impl ProtoFmt for Req {
    type Proto = proto::PushBatchVotes;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut votes = vec![];
        for (i, e) in r.votes.iter().enumerate() {
            votes.push(Arc::new(
                ProtoFmt::read(e).with_context(|| format!("votes[{i}]"))?,
            ));
        }
        Ok(Self(votes))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            votes: self.0.iter().map(|a| ProtoFmt::build(a.as_ref())).collect(),
        }
    }
}
