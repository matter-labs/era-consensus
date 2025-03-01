//! This crate contains the consensus component, which is responsible for handling the logic that allows us to reach agreement on blocks.
//! It uses a new cosnensus algorithm developed at Matter Labs, called ChonkyBFT. You can find the specification of the algorithm [here](../../../../spec).

use std::sync::Arc;

use anyhow::Context;
pub use config::Config;
use zksync_concurrency::{
    ctx,
    error::Wrap as _,
    scope,
    sync::{self, prunable_mpsc::SelectionFunctionResult},
};
use zksync_consensus_roles::validator;

mod config;
mod metrics;
pub mod testonly;
mod v1_chonky_bft;
mod v2_chonky_bft;

// Renaming network messages for clarity.
#[allow(missing_docs)]
pub type ToNetworkMessage = zksync_consensus_network::io::ConsensusInputMessage;
#[allow(missing_docs)]
pub type FromNetworkMessage = zksync_consensus_network::io::ConsensusReq;

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

impl Config {
    /// Starts the bft component. It will start running, processing incoming messages and
    /// sending output messages.
    pub async fn run(
        self,
        ctx: &ctx::Ctx,
        outbound_channel: ctx::channel::UnboundedSender<ToNetworkMessage>,
        inbound_channel: sync::prunable_mpsc::Receiver<FromNetworkMessage>,
    ) -> anyhow::Result<()> {
        let genesis = self.block_store.genesis();

        anyhow::ensure!(
            validator::ProtocolVersion::compatible(&genesis.protocol_version),
            "Incompatible protocol version. Genesis protocol version: {:?}.",
            genesis.protocol_version
        );
        genesis.verify().context("genesis().verify()")?;

        if let Some(prev) = genesis.first_block.prev() {
            tracing::info!("Waiting for the pre-fork blocks to be persisted.");
            if let Err(ctx::Canceled) = self.block_store.wait_until_persisted(ctx, prev).await {
                return Ok(());
            }
        }

        // Get the protocol version from genesis and start the corresponding state machine.
        match genesis.protocol_version {
            validator::ProtocolVersion(1) => {
                self.run_v1(ctx, outbound_channel, inbound_channel).await
            }
            validator::ProtocolVersion(2) => {
                self.run_v2(ctx, outbound_channel, inbound_channel).await
            }
            _ => anyhow::bail!(
                "Unsupported protocol version: {:?}",
                genesis.protocol_version
            ),
        }
    }

    async fn run_v1(
        self,
        ctx: &ctx::Ctx,
        outbound_channel: ctx::channel::UnboundedSender<ToNetworkMessage>,
        inbound_channel: sync::prunable_mpsc::Receiver<FromNetworkMessage>,
    ) -> anyhow::Result<()> {
        let cfg = Arc::new(self);

        let (proposer_sender, proposer_receiver) = sync::watch::channel(None);
        let replica = v1_chonky_bft::StateMachine::start(
            ctx,
            cfg.clone(),
            outbound_channel.clone(),
            inbound_channel,
            proposer_sender,
        )
        .await?;

        let res = scope::run!(ctx, |ctx, s| async {
            tracing::info!(
                "Starting consensus component (v1). Validator public key: {:?}.",
                cfg.secret_key.public()
            );

            s.spawn(async { replica.run(ctx).await.wrap("replica.run()") });
            s.spawn_bg(async {
                v1_chonky_bft::proposer::run_proposer(
                    ctx,
                    cfg.clone(),
                    outbound_channel,
                    proposer_receiver,
                )
                .await
                .wrap("run_proposer()")
            });

            Ok(())
        })
        .await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }

    async fn run_v2(
        self,
        ctx: &ctx::Ctx,
        outbound_channel: ctx::channel::UnboundedSender<ToNetworkMessage>,
        inbound_channel: sync::prunable_mpsc::Receiver<FromNetworkMessage>,
    ) -> anyhow::Result<()> {
        let cfg = Arc::new(self);

        let (proposer_sender, proposer_receiver) = sync::watch::channel(None);
        let replica = v2_chonky_bft::StateMachine::start(
            ctx,
            cfg.clone(),
            outbound_channel.clone(),
            inbound_channel,
            proposer_sender,
        )
        .await?;

        let res = scope::run!(ctx, |ctx, s| async {
            tracing::info!(
                "Starting consensus component (v2). Validator public key: {:?}.",
                cfg.secret_key.public()
            );

            s.spawn(async { replica.run(ctx).await.wrap("replica.run()") });
            s.spawn_bg(async {
                v2_chonky_bft::proposer::run_proposer(
                    ctx,
                    cfg.clone(),
                    outbound_channel,
                    proposer_receiver,
                )
                .await
                .wrap("run_proposer()")
            });

            Ok(())
        })
        .await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}

/// Creates a new input channel for the network messages.
pub fn create_input_channel() -> (
    sync::prunable_mpsc::Sender<FromNetworkMessage>,
    sync::prunable_mpsc::Receiver<FromNetworkMessage>,
) {
    sync::prunable_mpsc::channel(inbound_filter_predicate, inbound_selection_function)
}

/// Filter predicate for incoming messages.
fn inbound_filter_predicate(new_req: &FromNetworkMessage) -> bool {
    // Verify message signature
    new_req.msg.verify().is_ok()
}

/// Selection function for incoming messages.
fn inbound_selection_function(
    old_req: &FromNetworkMessage,
    new_req: &FromNetworkMessage,
) -> SelectionFunctionResult {
    if old_req.msg.key != new_req.msg.key || old_req.msg.msg.label() != new_req.msg.msg.label() {
        SelectionFunctionResult::Keep
    } else {
        // Discard older message
        if old_req.msg.msg.view().number < new_req.msg.msg.view().number {
            SelectionFunctionResult::DiscardOld
        } else {
            SelectionFunctionResult::DiscardNew
        }
    }
}
