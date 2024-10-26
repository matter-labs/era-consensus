//! This crate implements the ChonkyBFT algorithm. You can find the specification of the algorithm [here](../../../../spec).

use crate::io::{InputMessage, OutputMessage};
use anyhow::Context;
pub use config::Config;
use std::sync::Arc;
use tracing::Instrument;
use zksync_concurrency::{ctx, error::Wrap as _, scope};
use zksync_consensus_roles::validator;
use zksync_consensus_utils::pipe::ActorPipe;

pub(crate) mod chonky_bft;
mod config;
pub mod io;
mod metrics;
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
            tracing::info!("Waiting for the pre-fork blocks to be persisted");
            if let Err(ctx::Canceled) = self.block_store.wait_until_persisted(ctx, prev).await {
                return Ok(());
            }
        }

        let cfg = Arc::new(self);
        let (replica, replica_send) =
            chonky_bft::StateMachine::start(ctx, cfg.clone(), pipe.send.clone()).await?;

        let res = scope::run!(ctx, |ctx, s| async {
            let justification_recv = replica.justification_watch.subscribe();

            s.spawn_bg(async { replica.run(ctx).await.wrap("replica.run()") });
            s.spawn_bg(async {
                chonky_bft::proposer::run_proposer(ctx, cfg.clone(), pipe.send, justification_recv)
                    .await
                    .wrap("run_proposer()")
            });

            tracing::info!("Starting consensus actor {:?}", cfg.secret_key.public());

            // This is the infinite loop where the consensus actually runs. The validator waits for
            // a message from the network and processes it accordingly.
            loop {
                async {
                    let InputMessage::Network(msg) = pipe
                        .recv
                        .recv(ctx)
                        .instrument(tracing::info_span!("wait_for_message"))
                        .await?;

                    replica_send.send(msg);

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
