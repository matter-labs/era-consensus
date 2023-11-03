//! Defines RPC for synchronizing blocks.

use crate::{io, mux};
use anyhow::Context;
use concurrency::{limiter, time};
use roles::validator::{BlockNumber, FinalBlock};
use schema::proto::network::gossip as proto;
use zksync_protobuf::{read_required, ProtoFmt};

/// `get_sync_state` RPC.
#[derive(Debug)]
pub(crate) struct PushSyncStateRpc;

impl super::Rpc for PushSyncStateRpc {
    const CAPABILITY_ID: mux::CapabilityId = 3;
    const INFLIGHT: u32 = 1;
    const RATE: limiter::Rate = limiter::Rate {
        burst: 1,
        refresh: time::Duration::seconds(5),
    };
    const METHOD: &'static str = "push_sync_state";

    type Req = io::SyncState;
    type Resp = SyncStateResponse;
}

impl ProtoFmt for io::SyncState {
    type Proto = proto::SyncState;

    fn read(message: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            first_stored_block: read_required(&message.first_stored_block)
                .context("first_stored_block")?,
            last_contiguous_stored_block: read_required(&message.last_contiguous_stored_block)
                .context("last_contiguous_stored_block")?,
            last_stored_block: read_required(&message.last_stored_block)
                .context("last_stored_block")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            first_stored_block: Some(self.first_stored_block.build()),
            last_contiguous_stored_block: Some(self.last_contiguous_stored_block.build()),
            last_stored_block: Some(self.last_stored_block.build()),
        }
    }

    fn max_size() -> usize {
        // TODO: estimate maximum size more precisely
        100 * zksync_protobuf::kB
    }
}

/// Response to [`SyncState`] acknowledging its processing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SyncStateResponse;

impl ProtoFmt for SyncStateResponse {
    type Proto = proto::SyncStateResponse;

    fn read(_message: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self)
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {}
    }

    fn max_size() -> usize {
        zksync_protobuf::kB
    }
}

/// `get_block` RPC.
#[derive(Debug)]
pub(crate) struct GetBlockRpc;

// TODO: determine more precise `INFLIGHT` / `RATE` values as a result of load testing
impl super::Rpc for GetBlockRpc {
    const CAPABILITY_ID: mux::CapabilityId = 4;
    const INFLIGHT: u32 = 5;
    const RATE: limiter::Rate = limiter::Rate {
        burst: 10,
        refresh: time::Duration::milliseconds(100),
    };
    const METHOD: &'static str = "get_block";

    type Req = GetBlockRequest;
    type Resp = GetBlockResponse;
}

/// Asks the server to send a block (including its transactions).
#[derive(Debug)]
pub(crate) struct GetBlockRequest(pub(crate) BlockNumber);

impl ProtoFmt for GetBlockRequest {
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

impl ProtoFmt for io::GetBlockError {
    type Proto = proto::get_block_response::Error;

    fn read(message: &Self::Proto) -> anyhow::Result<Self> {
        use proto::get_block_response::ErrorReason;

        let reason = message.reason.context("missing reason")?;
        let reason = ErrorReason::try_from(reason).context("reason")?;
        Ok(match reason {
            ErrorReason::NotSynced => Self::NotSynced,
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::get_block_response::ErrorReason;

        Self::Proto {
            reason: Some(match self {
                Self::NotSynced => ErrorReason::NotSynced as i32,
            }),
        }
    }

    fn max_size() -> usize {
        zksync_protobuf::kB
    }
}

/// Response to a [`GetBlockRequest`] containing a block or a reason it cannot be retrieved.
#[derive(Debug)]
pub(crate) struct GetBlockResponse(pub(crate) io::GetBlockResponse);

impl From<io::GetBlockResponse> for GetBlockResponse {
    fn from(response: io::GetBlockResponse) -> Self {
        Self(response)
    }
}

impl ProtoFmt for GetBlockResponse {
    type Proto = proto::GetBlockResponse;

    fn read(message: &Self::Proto) -> anyhow::Result<Self> {
        use proto::get_block_response::Result as GetBlockResult;

        let result = message.result.as_ref().context("missing result")?;
        let result = match result {
            GetBlockResult::Block(block) => Ok(FinalBlock::read(block).context("block")?),
            GetBlockResult::Error(error) => Err(io::GetBlockError::read(error).context("error")?),
        };
        Ok(Self(result))
    }

    fn build(&self) -> Self::Proto {
        use proto::get_block_response::Result as GetBlockResult;

        let result = match &self.0 {
            Ok(block) => GetBlockResult::Block(block.build()),
            Err(err) => GetBlockResult::Error(err.build()),
        };
        Self::Proto {
            result: Some(result),
        }
    }

    fn max_size() -> usize {
        zksync_protobuf::MB
    }
}
