//! # Consensus
//! This crate implements the Fastest-HotStuff algorithm that is described in an upcoming paper
//! It is a two-phase unchained consensus with quadratic view change (in number of authenticators, in number of
//! messages it is linear) and optimistic responsiveness.
//!
//! ## Node set
//! Right now, we assume that we have a static node set. In other words, we are running in proof-of-authority. When this repo is updated
//! to proof-of-stake, we will have a dynamic node set.
//!
//! ## Resources
//! - [Fast-HotStuff paper](https://arxiv.org/pdf/2010.11454.pdf)
//! - [HotStuff paper](https://arxiv.org/pdf/1803.05069.pdf)
//! - [HotStuff-2 paper](https://eprint.iacr.org/2023/397.pdf)
//! - [Notes on modern consensus algorithms](https://timroughgarden.github.io/fob21/andy.pdf)
//! - [Blog post comparing several consensus algorithms](https://decentralizedthoughts.github.io/2023-04-01-hotstuff-2/)
//! - Blog posts explaining [safety](https://seafooler.com/2022/01/24/understanding-safety-hotstuff/) and [responsiveness](https://seafooler.com/2022/04/02/understanding-responsiveness-hotstuff/)

use crate::io::{InputMessage, OutputMessage};
use concurrency::ctx;
use inner::ConsensusInner;
use roles::validator;
use std::sync::Arc;
use storage::Storage;
use tracing::{info, instrument};
use utils::pipe::ActorPipe;

mod inner;
pub mod io;
mod leader;
mod metrics;
pub mod misc;
mod replica;
pub mod testonly;
#[cfg(test)]
mod tests;

/// The Consensus struct implements the consensus algorithm and is the main entry point for the consensus actor.
#[derive(Debug)]
pub struct Consensus {
    /// The inner struct contains the data that is shared between the consensus state machines.
    pub(crate) inner: ConsensusInner,
    /// The replica state machine.
    pub(crate) replica: replica::StateMachine,
    /// The leader state machine.
    pub(crate) leader: leader::StateMachine,
}

impl Consensus {
    /// Creates a new Consensus struct.
    #[instrument(level = "trace", ret)]
    pub fn new(
        ctx: &ctx::Ctx,
        pipe: ActorPipe<InputMessage, OutputMessage>,
        secret_key: validator::SecretKey,
        validator_set: validator::ValidatorSet,
        storage: Arc<Storage>,
    ) -> Self {
        Consensus {
            inner: ConsensusInner {
                pipe,
                secret_key,
                validator_set,
            },
            replica: replica::StateMachine::new(storage),
            leader: leader::StateMachine::new(ctx),
        }
    }

    /// Starts the Consensus actor. It will start running, processing incoming messages and
    /// sending output messages. This is a blocking method.
    #[instrument(level = "trace", ret)]
    pub fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        info!(
            "Starting consensus actor {:?}",
            self.inner.secret_key.public()
        );

        // We need to start the replica before processing inputs.
        self.replica.start(ctx, &self.inner);

        // This is the infinite loop where the consensus actually runs. The validator waits for either
        // a message from the network or for a timeout, and processes each accordingly.
        loop {
            let input = self
                .inner
                .pipe
                .recv(&ctx.with_deadline(self.replica.timeout_deadline))
                .block()
                .ok();

            // We check if the context is active before processing the input. If the context is not active,
            // we stop.
            if !ctx.is_active() {
                return Ok(());
            }

            match input {
                Some(InputMessage::Network(req)) => {
                    match &req.msg.msg {
                        validator::ConsensusMsg::ReplicaPrepare(_)
                        | validator::ConsensusMsg::ReplicaCommit(_) => {
                            self.leader.process_input(ctx, &self.inner, req.msg)
                        }
                        validator::ConsensusMsg::LeaderPrepare(_)
                        | validator::ConsensusMsg::LeaderCommit(_) => {
                            self.replica.process_input(ctx, &self.inner, Some(req.msg))
                        }
                    }
                    // Notify network actor that the message has been processed.
                    // Ignore sending error.
                    let _ = req.ack.send(());
                }
                None => self.replica.process_input(ctx, &self.inner, None),
            }
        }
    }
}
