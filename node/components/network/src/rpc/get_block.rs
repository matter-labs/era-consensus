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

impl Resp {
    /// Clears pregenesis data from the response.
    /// Use to simulate node behavior before pre-genesis support.
    pub(crate) fn clear_pregenesis_data(&mut self) {
        if let Some(validator::Block::PreGenesis(_)) = &self.0 {
            self.0 = None;
        }
    }
}

impl ProtoFmt for Resp {
    type Proto = proto::GetBlockResponse;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use validator::Block as B;
        let block = r
            .block
            .as_ref()
            .map(ProtoFmt::read)
            .transpose()
            .context("block")?
            .map(B::FinalV1);
        let pregenesis = r
            .pre_genesis
            .as_ref()
            .map(ProtoFmt::read)
            .transpose()
            .context("pre_genesis")?
            .map(B::PreGenesis);
        Ok(Self(block.or(pregenesis)))
    }

    fn build(&self) -> Self::Proto {
        use validator::Block as B;
        let mut p = Self::Proto::default();
        match self.0.as_ref() {
            Some(B::FinalV1(b)) => p.block = Some(b.build()),
            Some(B::PreGenesis(b)) => p.pre_genesis = Some(b.build()),
            None => {}
        }
        p
    }
}
