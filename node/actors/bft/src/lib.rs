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
use anyhow::Context as _;
use inner::ConsensusInner;
use std::sync::Arc;
use tracing::{info, instrument};
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;
use zksync_consensus_storage::ReplicaStore;
use zksync_consensus_utils::pipe::ActorPipe;

mod inner;
pub mod io;
mod leader;
mod metrics;
pub mod misc;
mod replica;
pub mod testonly;
#[cfg(test)]
mod tests;

/// Payload provider for the new blocks.
#[async_trait::async_trait]
pub trait PayloadSource: Send + Sync + 'static {
    /// Propose a payload for the block `block_number`.
    async fn propose(
        &self,
        ctx: &ctx::Ctx,
        block_number: validator::BlockNumber,
    ) -> anyhow::Result<validator::Payload>;
}

/// The Consensus struct implements the consensus algorithm and is the main entry point for the consensus actor.
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
    #[instrument(level = "trace", skip(payload_source))]
    pub async fn new(
        ctx: &ctx::Ctx,
        pipe: ActorPipe<InputMessage, OutputMessage>,
        protocol_version: validator::ProtocolVersion,
        secret_key: validator::SecretKey,
        validator_set: validator::ValidatorSet,
        storage: ReplicaStore,
        payload_source: Arc<dyn PayloadSource>,
    ) -> anyhow::Result<Self> {
        Ok(Consensus {
            inner: ConsensusInner {
                pipe,
                secret_key,
                validator_set,
                protocol_version,
            },
            replica: replica::StateMachine::new(ctx, storage).await?,
            leader: leader::StateMachine::new(ctx, payload_source),
        })
    }

    /// Starts the Consensus actor. It will start running, processing incoming messages and
    /// sending output messages. This is a blocking method.
    #[instrument(level = "trace", skip(self) ret)]
    pub async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        info!(
            "Starting consensus actor {:?}",
            self.inner.secret_key.public()
        );

        // We need to start the replica before processing inputs.
        self.replica
            .start(ctx, &self.inner)
            .await
            .context("replica.start()")?;

        // This is the infinite loop where the consensus actually runs. The validator waits for either
        // a message from the network or for a timeout, and processes each accordingly.
        loop {
            let input = self
                .inner
                .pipe
                .recv(&ctx.with_deadline(self.replica.timeout_deadline))
                .await
                .ok();

            // We check if the context is active before processing the input. If the context is not active,
            // we stop.
            if !ctx.is_active() {
                return Ok(());
            }

            match input {
                Some(InputMessage::Network(req)) => {
                    if req.msg.msg.protocol_version() != self.inner.protocol_version {
                        tracing::warn!(
                            "bad protocol version (expected: {:?}, received: {:?})",
                            self.inner.protocol_version,
                            req.msg.msg.protocol_version()
                        );
                        continue;
                    }
                    match &req.msg.msg {
                        validator::ConsensusMsg::ReplicaPrepare(_)
                        | validator::ConsensusMsg::ReplicaCommit(_) => {
                            self.leader.process_input(ctx, &self.inner, req.msg).await?;
                        }
                        validator::ConsensusMsg::LeaderPrepare(_)
                        | validator::ConsensusMsg::LeaderCommit(_) => {
                            self.replica
                                .process_input(ctx, &self.inner, Some(req.msg))
                                .await?;
                        }
                    }
                    // Notify network actor that the message has been processed.
                    // Ignore sending error.
                    let _ = req.ack.send(());
                }
                None => {
                    self.replica.process_input(ctx, &self.inner, None).await?;
                }
            }
        }
    }
}
