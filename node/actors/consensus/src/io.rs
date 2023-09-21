//! Input and output messages for the Consensus actor. These are processed by the executor actor.

use network::io::{ConsensusInputMessage, ConsensusReq};
use roles::validator;

/// All the messages that other actors can send to the Consensus actor.
#[derive(Debug)]
pub enum InputMessage {
    /// Message types from the Network actor.
    Network(ConsensusReq),
}

/// All the messages that the Consensus actor sends to other actors.
#[derive(Debug, PartialEq)]
pub enum OutputMessage {
    /// Message types to the Network actor.
    Network(ConsensusInputMessage),
    /// Message types to the Sync actor.
    FinalizedBlock(validator::FinalBlock),
}

impl From<ConsensusInputMessage> for OutputMessage {
    fn from(message: ConsensusInputMessage) -> Self {
        Self::Network(message)
    }
}
