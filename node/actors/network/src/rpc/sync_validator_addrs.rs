//! Defines an Rpc for synchronizing ValidatorAddrs data.
use crate::mux;
use anyhow::Context as _;
use std::sync::Arc;
use zksync_concurrency::{limiter, time};
use zksync_consensus_roles::validator;
use zksync_consensus_schema as schema;
use zksync_consensus_schema::{proto::network::gossip as proto, ProtoFmt};

/// SyncValidatorAddrs Rpc.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 1;
    const INFLIGHT: u32 = 1;
    const RATE: limiter::Rate = limiter::Rate {
        burst: 1,
        refresh: time::Duration::seconds(5),
    };
    const METHOD: &'static str = "sync_validator_addrs";

    type Req = Req;
    type Resp = Resp;
}

/// Ask server to send ValidatorAddrs update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req;

/// Response to SyncValidatorAddrsReq.
/// Contains a batch of new ValidatorAddrs that server
/// learned since the client's last SyncValidatorAddrs Rpc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resp(pub(crate) Vec<Arc<validator::Signed<validator::NetAddress>>>);

impl ProtoFmt for Req {
    type Proto = proto::SyncValidatorAddrsReq;

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

impl ProtoFmt for Resp {
    type Proto = proto::SyncValidatorAddrsResp;

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
        schema::MB
    }
}
