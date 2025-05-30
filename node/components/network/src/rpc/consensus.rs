//! Defines RPC for passing consensus messages.
use zksync_consensus_roles::validator;
use zksync_protobuf::{read_required, ProtoFmt};

use super::Capability;
use crate::proto::consensus as proto;

/// Consensus RPC.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY: Capability = Capability::Consensus;
    const INFLIGHT: u32 = 3;
    const METHOD: &'static str = "consensus";
    type Req = Req;
    type Resp = Resp;

    fn submethod(req: &Self::Req) -> &'static str {
        req.0.msg.label()
    }
}

/// Signed consensus message that the receiving peer should process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req(pub(crate) validator::Signed<validator::ConsensusMsg>);

/// Confirmation that the consensus message has been processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resp;

impl ProtoFmt for Req {
    type Proto = proto::ConsensusReq;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        read_required(&r.msg).map(Self)
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            msg: Some(self.0.build()),
        }
    }
}

impl ProtoFmt for Resp {
    type Proto = proto::ConsensusResp;

    fn read(_r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self)
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {}
    }
}
