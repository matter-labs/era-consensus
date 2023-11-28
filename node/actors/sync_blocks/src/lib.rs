//! # Sync Blocks Actor
//!
//! This crate contains an actor implementing block syncing among nodes, which is tied to the gossip
//! network RPCs.
use crate::{
    io::{InputMessage, OutputMessage},
    message_handler::SyncBlocksMessageHandler,
};
use std::sync::Arc;
use tracing::instrument;
use zksync_concurrency::{
    ctx, scope,
    sync::{self, watch},
};
use zksync_consensus_network::io::SyncState;
use zksync_consensus_storage::{StorageError, StorageResult, WriteBlockStore};
use zksync_consensus_utils::pipe::ActorPipe;

mod config;
pub mod io;
mod message_handler;
mod peers;
#[cfg(test)]
mod tests;

pub use crate::config::Config;
use crate::peers::PeerStates;

/// Block syncing actor responsible for synchronizing L2 blocks with other nodes.
#[derive(Debug)]
pub struct SyncBlocks {
    /// Part of the actor responsible for handling inbound messages.
    pub(crate) message_handler: SyncBlocksMessageHandler,
    /// Peer states.
    pub(crate) peer_states: PeerStates,
    /// Sender of `SyncState` updates.
    pub(crate) state_sender: watch::Sender<SyncState>,
}

impl SyncBlocks {
    /// Creates a new actor.
    pub async fn new(
        ctx: &ctx::Ctx,
        pipe: ActorPipe<InputMessage, OutputMessage>,
        storage: Arc<dyn WriteBlockStore>,
        config: Config,
    ) -> anyhow::Result<Self> {
        let (state_sender, _) = watch::channel(Self::get_sync_state(ctx, storage.as_ref()).await?);
        let (peer_states, peer_states_handle) = PeerStates::new(pipe.send, storage.clone(), config);
        let inner = SyncBlocksMessageHandler {
            message_receiver: pipe.recv,
            storage,
            peer_states_handle,
        };
        Ok(Self {
            message_handler: inner,
            peer_states,
            state_sender,
        })
    }

    /// Subscribes to `SyncState` updates emitted by the actor.
    pub fn subscribe_to_state_updates(&self) -> watch::Receiver<SyncState> {
        self.state_sender.subscribe()
    }

    /// Runs the actor processing incoming requests until `ctx` is canceled.
    #[instrument(level = "trace", skip_all, err)]
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let storage = self.message_handler.storage.clone();

        let result = scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(Self::emit_state_updates(ctx, storage, &self.state_sender));
            s.spawn_bg(self.peer_states.run(ctx));
            self.message_handler.process_messages(ctx).await
        })
        .await;

        // Since we clearly type cancellation errors, it's easier propagate them up to this entry point,
        // rather than catching in the constituent tasks.
        result.or_else(|err| match err {
            StorageError::Canceled(_) => Ok(()), // Cancellation is not propagated as an error
            StorageError::Internal(err) => Err(err),
        })
    }

    #[instrument(level = "trace", skip_all, err)]
    async fn emit_state_updates(
        ctx: &ctx::Ctx,
        storage: Arc<dyn WriteBlockStore>,
        state_sender: &watch::Sender<SyncState>,
    ) -> StorageResult<()> {
        let mut storage_subscriber = storage.subscribe_to_block_writes();
        loop {
            let state = Self::get_sync_state(ctx, storage.as_ref()).await?;
            if state_sender.send(state).is_err() {
                tracing::info!("`SyncState` subscriber dropped; exiting");
                return Ok(());
            }

            let block_number = *sync::changed(ctx, &mut storage_subscriber).await?;
            tracing::trace!(%block_number, "Received block write update");
        }
    }

    /// Gets the current sync state of this node based on information from the storage.
    #[instrument(level = "trace", skip_all)]
    async fn get_sync_state(
        ctx: &ctx::Ctx,
        storage: &dyn WriteBlockStore,
    ) -> StorageResult<SyncState> {
        let last_contiguous_block_number = storage.last_contiguous_block_number(ctx).await?;
        let last_contiguous_stored_block = storage
            .block(ctx, last_contiguous_block_number)
            .await?
            .expect("`last_contiguous_stored_block` disappeared");

        Ok(SyncState {
            first_stored_block: storage.first_block(ctx).await?.justification,
            last_contiguous_stored_block: last_contiguous_stored_block.justification,
            last_stored_block: storage.head_block(ctx).await?.justification,
        })
    }
}
