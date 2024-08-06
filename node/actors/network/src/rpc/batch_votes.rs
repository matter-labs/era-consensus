//! Defines RPC for passing consensus messages.
use crate::{mux, proto::gossip as proto};
use anyhow::Context as _;
use std::sync::Arc;
use zksync_consensus_roles::attester;
use zksync_protobuf::ProtoFmt;

/// RPC pushing fresh batch votes. 
pub(crate) struct PushRpc;

/// RPC requesting all batch votes from peers.
pub(crate) struct PullRpc;

/// Deprecated, because adding `genesis_hash` to `attester::Batch`
/// was not backward compatible - old binaries couldn't verify
/// signatures on messages with `genesis_hash` and were treating it
/// as malicious behavior.
#[allow(dead_code)]
pub(super) const PUSH_V1: mux::CapabilityId = 5;

/// Current version.
pub(super) const PUSH_V2: mux::CapabilityId = 8;

impl super::Rpc for PushRpc {
    const CAPABILITY_ID: mux::CapabilityId = PUSH_V2;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "push_batch_votes";
    type Req = Msg;
    type Resp = ();
}

impl super::Rpc for PullRpc {
    const CAPABILITY_ID: mux::CapabilityId = 9;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "pull_batch_votes";
    type Req = ();
    type Resp = Msg;
}

/// Signed batch message that the receiving peer should process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Msg(pub(crate) Vec<Arc<attester::Signed<attester::Batch>>>);

impl ProtoFmt for Msg {
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
