#![allow(missing_docs)]
use zksync_concurrency::oneshot;
use zksync_consensus_roles::validator;

/// All the messages that other actors can send to the Network actor.
#[derive(Debug)]
pub enum InputMessage {
    /// Message types from the Consensus actor.
    Consensus(ConsensusInputMessage),
}

/// Message types from the Consensus actor.
#[derive(Debug, PartialEq)]
pub struct ConsensusInputMessage {
    pub message: validator::Signed<validator::ConsensusMsg>,
}

impl From<ConsensusInputMessage> for InputMessage {
    fn from(message: ConsensusInputMessage) -> Self {
        Self::Consensus(message)
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

/// All the messages that the Network actor sends to other actors.
#[derive(Debug)]
pub enum OutputMessage {
    /// Message to the Consensus actor.
    Consensus(ConsensusReq),
}
