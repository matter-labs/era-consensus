//! Defines an Rpc for synchronizing ValidatorAddrs data.
use crate::{mux, proto::gossip as proto};
use anyhow::Context as _;
use std::sync::Arc;
use zksync_concurrency::{limiter, time};
use zksync_consensus_roles::validator;
use zksync_protobuf::ProtoFmt;

/// SyncValidatorAddrs Rpc.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 1;
    const INFLIGHT: u32 = 1;
    const RATE: limiter::Rate = limiter::Rate {
        burst: 1,
        refresh: time::Duration::seconds(5),
    };
    const METHOD: &'static str = "push_validator_addrs";

    type Req = Req;
    type Resp = ();
}

/// Contains a batch of new ValidatorAddrs that server
/// learned since the client's last SyncValidatorAddrs Rpc.
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

    fn max_size() -> usize {
        zksync_protobuf::MB
    }
}
