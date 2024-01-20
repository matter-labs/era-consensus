//! RPC for fetching a block from peer.
use crate::{mux, proto::gossip as proto};
use anyhow::Context;
use zksync_concurrency::{limiter, time};
use zksync_consensus_roles::validator::{BlockNumber, FinalBlock};
use zksync_protobuf::{read_optional, ProtoFmt};

/// `get_block` RPC.
#[derive(Debug)]
pub(crate) struct Rpc;

// TODO: determine more precise `INFLIGHT` / `RATE` values as a result of load testing
impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 4;
    const INFLIGHT: u32 = 5;
    const RATE: limiter::Rate = limiter::Rate {
        burst: 10,
        refresh: time::Duration::milliseconds(100),
    };
    const METHOD: &'static str = "get_block";

    type Req = Req;
    type Resp = Resp;
}

/// Asks the server to send a block (including its transactions).
#[derive(Debug, PartialEq)]
pub(crate) struct Req(pub(crate) BlockNumber);

impl ProtoFmt for Req {
    type Proto = proto::GetBlockRequest;

    fn read(message: &Self::Proto) -> anyhow::Result<Self> {
        let number = message.number.context("number")?;
        Ok(Self(BlockNumber(number)))
    }

    fn build(&self) -> Self::Proto {
        let BlockNumber(number) = self.0;
        Self::Proto {
            number: Some(number),
        }
    }

    fn max_size() -> usize {
        zksync_protobuf::kB
    }
}

/// Response to a [`GetBlockRequest`] containing a block or a reason it cannot be retrieved.
#[derive(Debug, PartialEq)]
pub(crate) struct Resp(pub(crate) Option<FinalBlock>);

impl ProtoFmt for Resp {
    type Proto = proto::GetBlockResponse;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(read_optional(&r.block).context("block")?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            block: self.0.as_ref().map(ProtoFmt::build),
        }
    }

    fn max_size() -> usize {
        4 * zksync_protobuf::MB
    }
}
