//! RPC for fetching a block from peer.
use super::Capability;
use crate::proto::gossip as proto;
use anyhow::Context;
use zksync_consensus_roles::validator;
use zksync_protobuf::ProtoFmt;

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
        use proto::get_block_response::T;
        use validator::Block as B;
        Ok(Self(match &r.t {
            None => None,
            Some(T::Block(b)) => Some(B::Final(ProtoFmt::read(b).context("block")?)),
            Some(T::PreGenesis(b)) => Some(B::PreGenesis(
                ProtoFmt::read(b).context("pre_genesis_block")?,
            )),
        }))
    }

    fn build(&self) -> Self::Proto {
        use proto::get_block_response::T;
        use validator::Block as B;
        Self::Proto {
            t: match self.0.as_ref() {
                None => None,
                Some(B::Final(b)) => Some(T::Block(b.build())),
                Some(B::PreGenesis(b)) => Some(T::PreGenesis(b.build())),
            },
        }
    }
}
