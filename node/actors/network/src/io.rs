#![allow(missing_docs)]
use zksync_concurrency::oneshot;
use zksync_consensus_roles::{node, validator};

/// All the messages that other actors can send to the Network actor.
#[derive(Debug)]
pub enum InputMessage {
    /// Message types from the Consensus actor.
    Consensus(ConsensusInputMessage),
    /// Message types from the Sync Blocks actor.
    SyncBlocks(SyncBlocksInputMessage),
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
        response: oneshot::Sender<Result<validator::FinalBlock,GetBlockError>>,
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

/// Current block sync state of a node sent in response to [`GetSyncStateRequest`].
#[derive(Debug, Clone, PartialEq)]
pub struct SyncState {
    pub first_stored_block: validator::CommitQC,
    pub last_stored_block: validator::CommitQC,
}

/// Projection of [`SyncState`] comprised of block numbers.
#[derive(Debug, Clone, Copy)]
pub struct SyncStateNumbers {
    pub first_stored_block: validator::BlockNumber,
    pub last_stored_block: validator::BlockNumber,
}

impl SyncState {
    /// Returns numbers for block QCs contained in this state.
    pub fn numbers(&self) -> SyncStateNumbers {
        SyncStateNumbers {
            first_stored_block: self.first_stored_block.message.proposal.number,
            last_stored_block: self.last_stored_block.message.proposal.number,
        }
    }
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
        state: Box<SyncState>,
        /// Acknowledgement response returned by the block syncing actor.
        // TODO: return an error in case of invalid `SyncState`?
        response: oneshot::Sender<()>,
    },
    /// Requests an L2 block with the specified number.
    GetBlock {
        /// Number of the block.
        block_number: validator::BlockNumber,
        /// Block returned by the block syncing actor.
        response: oneshot::Sender<Result<validator::FinalBlock,GetBlockError>>,
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
