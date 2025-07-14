//! RPC for fetching a block from peer.
use anyhow::Context;
use zksync_consensus_roles::validator;
use zksync_protobuf::ProtoFmt;

use super::Capability;
use crate::proto::gossip as proto;

/// `get_block` RPC.
#[derive(Debug)]
pub(crate) struct Rpc;

// TODO: determine more precise `INFLIGHT` / `RATE` values as a result of load testing
impl super::Rpc for Rpc {
    const CAPABILITY: Capability = Capability::GetBlock;
    const INFLIGHT: u32 = 5;
    const METHOD: &'static str = "get_block";

    type Req = Req;
    type Resp = Resp;
}

/// Asks the server to send a block (including its transactions).
#[derive(Debug, PartialEq)]
pub(crate) struct Req(pub(crate) validator::BlockNumber);

impl ProtoFmt for Req {
    type Proto = proto::GetBlockRequest;

    fn read(message: &Self::Proto) -> anyhow::Result<Self> {
        let number = message.number.context("number")?;
        Ok(Self(validator::BlockNumber(number)))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            number: Some(self.0 .0),
        }
    }
}

/// Response to a [`GetBlockRequest`] containing a block or a reason it cannot be retrieved.
#[derive(Debug, PartialEq)]
pub(crate) struct Resp(pub(crate) Option<validator::Block>);

impl ProtoFmt for Resp {
    type Proto = proto::GetBlockResponse;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use validator::Block as B;
        let block_v2 = r
            .block_v2
            .as_ref()
            .map(ProtoFmt::read)
            .transpose()
            .context("block_v2")?
            .map(B::FinalV2);

        let pregenesis = r
            .pre_genesis
            .as_ref()
            .map(ProtoFmt::read)
            .transpose()
            .context("pre_genesis")?
            .map(B::PreGenesis);

        Ok(Self(block_v2.or(pregenesis)))
    }

    fn build(&self) -> Self::Proto {
        use validator::Block as B;
        let mut p = Self::Proto::default();
        match self.0.as_ref() {
            Some(B::FinalV2(b)) => p.block_v2 = Some(b.build()),
            Some(B::PreGenesis(b)) => p.pre_genesis = Some(b.build()),
            None => {}
        }
        p
    }
}
