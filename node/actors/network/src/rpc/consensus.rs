//! Defines RPC for passing consensus messages.
use crate::mux;
use zksync_concurrency::{limiter, time};
use zksync_consensus_roles::validator;
use zksync_consensus_schema::{proto::network::consensus as proto, read_required, ProtoFmt};
use zksync_consensus_schema as schema;

/// Consensus RPC.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 0;
    const INFLIGHT: u32 = 3;
    const RATE: limiter::Rate = limiter::Rate {
        burst: 10,
        refresh: time::Duration::ZERO,
    };
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

    fn max_size() -> usize {
        schema::MB
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

    fn max_size() -> usize {
        schema::kB
    }
}
