//! RPC for fetching a batch from peer.
use crate::{mux, proto::gossip as proto};
use anyhow::Context;
use zksync_consensus_roles::attester;
use zksync_protobuf::{read_optional, ProtoFmt};

/// `get_batch` RPC.
#[derive(Debug)]
pub(crate) struct Rpc;

// TODO: determine more precise `INFLIGHT` / `RATE` values as a result of load testing
impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 6;
    const INFLIGHT: u32 = 5;
    const METHOD: &'static str = "get_batch";

    type Req = Req;
    type Resp = Resp;
}

/// Asks the server to send a batch (including its blocks).
#[derive(Debug, PartialEq)]
pub(crate) struct Req(pub(crate) attester::BatchNumber);

impl ProtoFmt for Req {
    type Proto = proto::GetBatchRequest;

    fn read(message: &Self::Proto) -> anyhow::Result<Self> {
        let number = message.number.context("number")?;
        Ok(Self(attester::BatchNumber(number)))
    }

    fn build(&self) -> Self::Proto {
        let attester::BatchNumber(number) = self.0;
        Self::Proto {
            number: Some(number),
        }
    }
}

/// Response to a [`GetBatchRequest`] containing a batch or a reason it cannot be retrieved.
#[derive(Debug, PartialEq)]
pub(crate) struct Resp(pub(crate) Option<attester::FinalBatch>);

impl ProtoFmt for Resp {
    type Proto = proto::GetBatchResponse;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(read_optional(&r.batch).context("batch")?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            batch: self.0.as_ref().map(ProtoFmt::build),
        }
    }
}
