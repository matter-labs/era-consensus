//! Defines RPC for passing consensus messages.
use std::sync::Arc;

use crate::{mux, proto::gossip as proto};
use anyhow::Context as _;
use zksync_consensus_roles::attester::{self, L1Batch};
use zksync_protobuf::ProtoFmt;

/// Signature RPC.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 5;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "push_signature";
    type Req = Req;
    type Resp = ();
}

/// Signed consensus message that the receiving peer should process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req(pub(crate) Vec<Arc<attester::SignedBatchMsg<L1Batch>>>);

impl ProtoFmt for Req {
    type Proto = proto::PushSignature;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut signatures = vec![];
        for (i, e) in r.signatures.iter().enumerate() {
            signatures.push(Arc::new(
                ProtoFmt::read(e).with_context(|| format!("signatures[{i}]"))?,
            ));
        }
        Ok(Self(signatures))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            signatures: self.0.iter().map(|a| ProtoFmt::build(a.as_ref())).collect(),
        }
    }
}
