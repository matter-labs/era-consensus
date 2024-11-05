#![allow(missing_docs)]
use zksync_concurrency::oneshot;
use zksync_consensus_roles::validator;

/// Message types from the Consensus actor.
#[derive(Debug, PartialEq)]
pub struct ConsensusInputMessage {
    pub message: validator::Signed<validator::ConsensusMsg>,
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
