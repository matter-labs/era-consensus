//! Input and output messages for the [`SyncBlocks`](crate::SyncBlocks) actor.

use network::io::{SyncBlocksInputMessage, SyncBlocksRequest};

/// All the messages that other actors can send to the `SyncBlocks` actor.
#[derive(Debug)]
pub enum InputMessage {
    /// Message types from the Network actor.
    Network(SyncBlocksRequest),
}

impl From<SyncBlocksRequest> for InputMessage {
    fn from(request: SyncBlocksRequest) -> Self {
        Self::Network(request)
    }
}

/// Messages produced by the `SyncBlocks` actor.
#[derive(Debug)]
pub enum OutputMessage {
    /// Message to the Network actor.
    Network(SyncBlocksInputMessage),
}

impl From<SyncBlocksInputMessage> for OutputMessage {
    fn from(message: SyncBlocksInputMessage) -> Self {
        Self::Network(message)
    }
}
