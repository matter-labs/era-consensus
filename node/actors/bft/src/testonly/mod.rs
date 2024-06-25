//! This module contains utilities that are only meant for testing purposes.

use crate::io::InputMessage;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use zksync_concurrency::oneshot;
use zksync_consensus_network::io::ConsensusReq;

mod make;
#[cfg(test)]
mod node;
#[cfg(test)]
mod run;
#[cfg(test)]
pub(crate) mod ut_harness;

pub use make::*;
#[cfg(test)]
pub(crate) use node::*;
#[cfg(test)]
pub(crate) use run::*;
#[cfg(test)]
pub mod twins;

// Generates a random InputMessage.
impl Distribution<InputMessage> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> InputMessage {
        let (send, _) = oneshot::channel();
        InputMessage::Network(ConsensusReq {
            msg: rng.gen(),
            ack: send,
        })
    }
}
