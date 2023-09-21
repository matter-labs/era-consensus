//! This module contains utilities that are only meant for testing purposes.

use crate::io::InputMessage;
use concurrency::oneshot;
use network::io::ConsensusReq;
use rand::{distributions::Standard, prelude::Distribution, Rng};

#[cfg(test)]
mod fuzz;
mod make;
#[cfg(test)]
mod node;
#[cfg(test)]
mod run;

#[cfg(test)]
pub(crate) use fuzz::*;
pub use make::*;
#[cfg(test)]
pub(crate) use node::*;
#[cfg(test)]
pub(crate) use run::*;

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
