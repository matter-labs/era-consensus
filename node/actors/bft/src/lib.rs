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
use anyhow::Context;
pub use config::Config;
use std::sync::Arc;
use tracing::Instrument;
use zksync_concurrency::{ctx, error::Wrap as _, oneshot, scope};
use zksync_consensus_network::io::ConsensusReq;
use zksync_consensus_roles::validator;
use zksync_consensus_utils::pipe::ActorPipe;

mod config;
pub mod io;
mod leader;
mod metrics;
mod replica;
pub mod testonly;
#[cfg(test)]
mod tests;

/// Protocol version of this BFT implementation.
pub const PROTOCOL_VERSION: validator::ProtocolVersion = validator::ProtocolVersion::CURRENT;

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
        let genesis = self.block_store.genesis();
        anyhow::ensure!(genesis.protocol_version == validator::ProtocolVersion::CURRENT);
        genesis.verify().context("genesis().verify()")?;

        if let Some(prev) = genesis.first_block.prev() {
            tracing::info!("Waiting for the pre-genesis blocks to be persisted");
            if let Err(ctx::Canceled) = self.block_store.wait_until_persisted(ctx, prev).await {
                return Ok(());
            }
        }

        let cfg = Arc::new(self);
        let (leader, leader_send) = leader::StateMachine::new(ctx, cfg.clone(), pipe.send.clone());
        let (replica, replica_send) =
            replica::StateMachine::start(ctx, cfg.clone(), pipe.send.clone()).await?;

        let res = scope::run!(ctx, |ctx, s| async {
            let prepare_qc_recv = leader.prepare_qc.subscribe();

            s.spawn_bg(async { replica.run(ctx).await.wrap("replica.run()") });
            s.spawn_bg(async { leader.run(ctx).await.wrap("leader.run()") });
            s.spawn_bg(async {
                leader::StateMachine::run_proposer(ctx, &cfg, prepare_qc_recv, &pipe.send)
                    .await
                    .wrap("run_proposer()")
            });

            tracing::info!("Starting consensus actor {:?}", cfg.secret_key.public());

            // This is the infinite loop where the consensus actually runs. The validator waits for either
            // a message from the network or for a timeout, and processes each accordingly.
            loop {
                async {
                    let InputMessage::Network(req) = pipe
                        .recv
                        .recv(ctx)
                        .instrument(tracing::info_span!("wait_for_message"))
                        .await?;
                    use validator::ConsensusMsg as M;
                    match &req.msg.msg {
                        M::ReplicaPrepare(_) => {
                            // This is a hacky way to do a clone. This is necessary since we don't want to derive
                            // Clone for ConsensusReq. When we change to ChonkyBFT this will be removed anyway.
                            let (ack, _) = oneshot::channel();
                            let new_req = ConsensusReq {
                                msg: req.msg.clone(),
                                ack,
                            };

                            replica_send.send(new_req);
                            leader_send.send(req);
                        }
                        M::ReplicaCommit(_) => leader_send.send(req),
                        M::LeaderPrepare(_) | M::LeaderCommit(_) => replica_send.send(req),
                    }

                    ctx::Ok(())
                }
                .instrument(tracing::info_span!("bft_iter"))
                .await?;
            }
        })
        .await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}
