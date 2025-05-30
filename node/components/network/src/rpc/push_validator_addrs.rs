//! RPC for synchronizing ValidatorAddrs data.
use std::sync::Arc;

use anyhow::Context as _;
use zksync_consensus_roles::validator;
use zksync_protobuf::ProtoFmt;

use super::Capability;
use crate::proto::gossip as proto;

/// PushValidatorAddrs RPC.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY: Capability = Capability::PushValidatorAddrs;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "push_validator_addrs";

    type Req = Req;
    type Resp = ();
}

/// Contains a batch of new ValidatorAddrs that the sender has learned about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req(pub(crate) Vec<Arc<validator::Signed<validator::NetAddress>>>);

impl ProtoFmt for Req {
    type Proto = proto::PushValidatorAddrs;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut addrs = vec![];
        for (i, e) in r.net_addresses.iter().enumerate() {
            addrs.push(Arc::new(
                ProtoFmt::read(e).with_context(|| format!("net_addresses[{i}]"))?,
            ));
        }
        Ok(Self(addrs))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            net_addresses: self.0.iter().map(|a| ProtoFmt::build(a.as_ref())).collect(),
        }
    }
}
