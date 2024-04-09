#![allow(missing_docs)]
use zksync_concurrency::oneshot;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::BlockStoreState;

/// All the messages that other actors can send to the Network actor.
#[derive(Debug)]
pub enum InputMessage {
    /// Message types from the Consensus actor.
    Consensus(ConsensusInputMessage),
    /// Message types from the Sync Blocks actor.
    SyncBlocks(SyncBlocksInputMessage),
    /// Message of type L1BatchSignatureMsg from the validator.
    L1Batch(L1BatchInputMessage),
}

#[derive(Debug)]
pub struct L1BatchInputMessage {
    pub message: validator::Signed<validator::L1BatchMsg>,
}

/// Message types from the Consensus actor.
#[derive(Debug, PartialEq)]
pub struct ConsensusInputMessage {
    pub message: validator::Signed<validator::ConsensusMsg>,
    pub recipient: Target,
}

impl From<ConsensusInputMessage> for InputMessage {
    fn from(message: ConsensusInputMessage) -> Self {
        Self::Consensus(message)
    }
}

/// Message types from the Sync Blocks actor.
#[derive(Debug)]
pub enum SyncBlocksInputMessage {
    /// Request to get a block from a specific peer.
    GetBlock {
        recipient: node::PublicKey,
        number: validator::BlockNumber,
        response: oneshot::Sender<Result<validator::FinalBlock, GetBlockError>>,
    },
}

impl From<SyncBlocksInputMessage> for InputMessage {
    fn from(message: SyncBlocksInputMessage) -> Self {
        Self::SyncBlocks(message)
    }
}

/// Consensus message received from the network.
#[derive(Debug)]
pub struct ConsensusReq {
    /// Payload.
    pub msg: validator::Signed<validator::ConsensusMsg>,
    /// Channel that should be used to notify network actor that
    /// processing of this message has been completed.
    /// Used for rate limiting.
    pub ack: oneshot::Sender<()>,
}

/// Error returned in response to [`GetBlock`] call.
///
/// Note that these errors don't include network-level errors, only app-level ones.
#[derive(Debug, thiserror::Error)]
pub enum GetBlockError {
    /// Transient error: the node doesn't have the requested L2 block.
    #[error("node doesn't have the requested L2 block")]
    NotAvailable,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug)]
pub enum SyncBlocksRequest {
    /// Notifies about an update in peer's `SyncState`.
    UpdatePeerSyncState {
        /// Peer that has reported the update.
        peer: node::PublicKey,
        /// Updated peer syncing state.
        state: BlockStoreState,
        /// Acknowledgement response returned by the block syncing actor.
        // TODO: return an error in case of invalid `SyncState`?
        response: oneshot::Sender<()>,
    },
}

/// All the messages that the Network actor sends to other actors.
#[derive(Debug)]
pub enum OutputMessage {
    /// Message to the Consensus actor.
    Consensus(ConsensusReq),
    /// Message to the block syncing actor.
    SyncBlocks(SyncBlocksRequest),
}

impl From<SyncBlocksRequest> for OutputMessage {
    fn from(request: SyncBlocksRequest) -> Self {
        Self::SyncBlocks(request)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Validator(validator::PublicKey),
    Broadcast,
}
