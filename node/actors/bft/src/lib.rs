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
use std::sync::Arc;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator;
use zksync_consensus_utils::pipe::ActorPipe;

mod config;
pub mod io;
mod leader;
mod metrics;
pub mod misc;
mod replica;
pub mod testonly;
#[cfg(test)]
mod tests;

pub use config::Config;

/// Protocol version of this BFT implementation.
pub const PROTOCOL_VERSION: validator::ProtocolVersion = validator::ProtocolVersion::EARLIEST;

/// Payload proposal and verification trait.
#[async_trait::async_trait]
pub trait PayloadManager: std::fmt::Debug + Send + Sync {
    /// Used by leader to propose a payload for the next block.
    async fn propose(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload>;
    /// Used by replica to verify a payload for the next block proposed by the leader.
    async fn verify(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
        payload: &validator::Payload,
    ) -> ctx::Result<()>;
}

/// Channel through which bft actor sends network messages.
pub(crate) type OutputSender = ctx::channel::UnboundedSender<OutputMessage>;

impl Config {
    /// Starts the bft actor. It will start running, processing incoming messages and
    /// sending output messages.
    pub async fn run(
        self,
        ctx: &ctx::Ctx,
        mut pipe: ActorPipe<InputMessage, OutputMessage>,
    ) -> anyhow::Result<()> {
        let cfg = Arc::new(self);
        let res = scope::run!(ctx, |ctx, s| async {
            let mut replica =
                replica::StateMachine::start(ctx, cfg.clone(), pipe.send.clone()).await?;
            let mut leader = leader::StateMachine::new(ctx, cfg.clone(), pipe.send.clone());

            s.spawn_bg(leader::StateMachine::run_proposer(
                ctx,
                &cfg,
                leader.prepare_qc.subscribe(),
                &pipe.send,
            ));

            tracing::info!("Starting consensus actor {:?}", cfg.secret_key.public());

            // This is the infinite loop where the consensus actually runs. The validator waits for either
            // a message from the network or for a timeout, and processes each accordingly.
            loop {
                let input = pipe
                    .recv
                    .recv(&ctx.with_deadline(replica.timeout_deadline))
                    .await
                    .ok();

                // We check if the context is active before processing the input. If the context is not active,
                // we stop.
                if !ctx.is_active() {
                    return Ok(());
                }

                let Some(InputMessage::Network(req)) = input else {
                    replica.start_new_view(ctx).await?;
                    continue;
                };

                use validator::ConsensusMsg as Msg;
                let res = match &req.msg.msg {
                    Msg::ReplicaPrepare(_) | Msg::ReplicaCommit(_) => {
                        leader.process_input(ctx, req.msg).await
                    }
                    Msg::LeaderPrepare(_) | Msg::LeaderCommit(_) => {
                        replica.process_input(ctx, req.msg).await
                    }
                };
                // Notify network actor that the message has been processed.
                // Ignore sending error.
                let _ = req.ack.send(());
                res?;
            }
        })
        .await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}
