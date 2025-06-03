//! This crate contains the consensus component, which is responsible for handling the logic that allows us to reach agreement on blocks.
//! It uses a new consensus algorithm developed at Matter Labs, called ChonkyBFT. You can find the specification of the algorithm [here](../../../../spec).

use std::sync::Arc;

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

impl Config {
    /// Starts the bft component. It will start running, processing incoming messages and
    /// sending output messages.
    pub async fn run(
        self,
        ctx: &ctx::Ctx,
        outbound_channel: ctx::channel::UnboundedSender<ToNetworkMessage>,
        inbound_channel: sync::prunable_mpsc::Receiver<FromNetworkMessage>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            validator::ProtocolVersion::compatible(&self.protocol_version()),
            "Incompatible protocol version. Genesis protocol version: {:?}.",
            self.protocol_version()
        );

        if let Some(prev) = self.first_block.prev() {
            tracing::trace!("Waiting for the pre-fork blocks to be persisted.");
            if let Err(ctx::Canceled) = self.engine_manager.wait_until_persisted(ctx, prev).await {
                return Ok(());
            }
        }

        // Clone the engine manager and epoch number to avoid moving them into the scope.
        // We need them to outlive the scope of the spawned tasks.
        let engine_manager = self.engine_manager.clone();
        let epoch_number = self.epoch;

        let res: ctx::Result<()> = scope::run!(ctx, |ctx, s| async {
            // Spawn task to terminate this instance of the bft component at the end of the current epoch.
            s.spawn(async {
                // Wait for the expiration block number for the current epoch.
                let expiration = engine_manager
                    .wait_for_validator_schedule_expiration(ctx, epoch_number)
                    .await?;

                // Wait until the expiration block is persisted.
                engine_manager.wait_until_persisted(ctx, expiration).await?;

                // When we already have the expiration block, we can stop the BFT component.
                s.cancel();

                Ok(())
            });

            // Get the protocol version from genesis and start the corresponding state machine.
            match self.protocol_version() {
                validator::ProtocolVersion(1) => self
                    .run_v1(ctx, outbound_channel, inbound_channel)
                    .await
                    .wrap("run_v1()"),
                validator::ProtocolVersion(2) => self
                    .run_v2(ctx, outbound_channel, inbound_channel)
                    .await
                    .wrap("run_v2()"),
                _ => Err(ctx::Error::Internal(anyhow::anyhow!(
                    "Unsupported protocol version: {:?}",
                    self.protocol_version()
                ))),
            }
        })
        .await;

        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }

    /// Starts the bft component with the v1 protocol version.
    async fn run_v1(
        self,
        ctx: &ctx::Ctx,
        outbound_channel: ctx::channel::UnboundedSender<ToNetworkMessage>,
        inbound_channel: sync::prunable_mpsc::Receiver<FromNetworkMessage>,
    ) -> ctx::Result<()> {
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

        scope::run!(ctx, |ctx, s| async {
            tracing::trace!(
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
        .await
    }

    /// Starts the bft component with the v2 protocol version.
    async fn run_v2(
        self,
        ctx: &ctx::Ctx,
        outbound_channel: ctx::channel::UnboundedSender<ToNetworkMessage>,
        inbound_channel: sync::prunable_mpsc::Receiver<FromNetworkMessage>,
    ) -> ctx::Result<()> {
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

        scope::run!(ctx, |ctx, s| async {
            tracing::trace!(
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
        .await
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
        // Discard older message.
        if old_req.msg.msg.view_number() < new_req.msg.msg.view_number() {
            SelectionFunctionResult::DiscardOld
        } else {
            SelectionFunctionResult::DiscardNew
        }
    }
}
