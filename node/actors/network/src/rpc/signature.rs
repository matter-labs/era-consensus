//! Defines RPC for passing consensus messages.
use crate::{consensus::Network, mux, proto::consensus as proto};
use zksync_consensus_roles::validator;
use zksync_protobuf::{read_required, ProtoFmt};

/// Signature RPC.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 5;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "signature";
    type Req = Req;
    type Resp = ();
}
pub(crate) struct L1BatchServer<'a>(pub(crate) &'a Network);
/// Signed consensus message that the receiving peer should process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req(pub(crate) validator::Signed<validator::L1BatchMsg>);

/// Confirmation that the signature message has been processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resp;

impl ProtoFmt for Req {
    type Proto = proto::SignatureReq;

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
    type Proto = proto::SignatureResp;

    fn read(_r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self)
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {}
    }
}
