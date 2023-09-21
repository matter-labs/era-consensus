//! # Sync Blocks Actor
//!
//! This crate contains an actor implementing block syncing among nodes, which is tied to the gossip
//! network RPCs.

use crate::{
    io::{InputMessage, OutputMessage},
    message_handler::SyncBlocksMessageHandler,
};
use concurrency::{
    ctx, scope,
    sync::{self, watch},
};
use network::io::SyncState;
use std::sync::Arc;
use storage::Storage;
use tracing::instrument;
use utils::pipe::ActorPipe;

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
    pub fn new(
        pipe: ActorPipe<InputMessage, OutputMessage>,
        storage: Arc<Storage>,
        config: Config,
    ) -> Self {
        let (state_sender, _) = watch::channel(Self::get_sync_state(&storage));
        let (peer_states, peer_states_handle) = PeerStates::new(pipe.send, storage.clone(), config);
        let inner = SyncBlocksMessageHandler {
            message_receiver: pipe.recv,
            storage,
            peer_states_handle,
        };
        Self {
            message_handler: inner,
            peer_states,
            state_sender,
        }
    }

    /// Subscribes to `SyncState` updates emitted by the actor.
    pub fn subscribe_to_state_updates(&self) -> watch::Receiver<SyncState> {
        self.state_sender.subscribe()
    }

    /// Runs the actor processing incoming requests until `ctx` is canceled.
    ///
    /// **This method is blocking and will run indefinitely.**
    #[instrument(level = "trace", skip_all, err)]
    pub fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let storage = self.message_handler.storage.clone();

        scope::run_blocking!(ctx, |ctx, s| {
            s.spawn_bg(Self::emit_state_updates(ctx, storage, &self.state_sender));
            s.spawn_bg(self.peer_states.run(ctx));
            self.message_handler.process_messages(ctx)
        })
    }

    #[instrument(level = "trace", skip_all, err)]
    async fn emit_state_updates(
        ctx: &ctx::Ctx,
        storage: Arc<Storage>,
        state_sender: &watch::Sender<SyncState>,
    ) -> anyhow::Result<()> {
        let mut storage_subscriber = storage.subscribe_to_block_writes();
        loop {
            let state = Self::get_sync_state(&storage);
            if state_sender.send(state).is_err() {
                tracing::info!("`SyncState` subscriber dropped; exiting");
                return Ok(());
            }

            let block_number = *sync::changed(ctx, &mut storage_subscriber).await?;
            tracing::trace!(%block_number, "Received block write update");
        }
    }

    /// Gets the current sync state of this node based on information from the storage.
    ///
    /// **This method is blocking.**
    #[instrument(level = "trace", skip_all)]
    fn get_sync_state(storage: &Storage) -> SyncState {
        let last_contiguous_block_number = storage.get_last_contiguous_block_number();
        let last_contiguous_stored_block = storage
            .get_block(last_contiguous_block_number)
            .expect("`last_contiguous_stored_block` disappeared");

        SyncState {
            first_stored_block: storage.get_first_block().justification,
            last_contiguous_stored_block: last_contiguous_stored_block.justification,
            last_stored_block: storage.get_head_block().justification,
        }
    }
}
