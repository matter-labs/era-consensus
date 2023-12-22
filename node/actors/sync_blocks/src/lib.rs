//! # Sync Blocks Actor
//!
//! This crate contains an actor implementing block syncing among nodes, which is tied to the gossip
//! network RPCs.
use crate::{
    io::{InputMessage, OutputMessage},
};
use std::sync::Arc;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_storage::BlockStore;
use zksync_consensus_utils::pipe::ActorPipe;
use zksync_consensus_network::io::{GetBlockError, SyncBlocksRequest};

mod config;
pub mod io;
mod peers;
#[cfg(test)]
mod tests;

pub use crate::config::Config;
use crate::peers::PeerStates;

/// Creates a new actor.
impl Config {
    pub async fn run(
        self,
        ctx: &ctx::Ctx,
        pipe: ActorPipe<InputMessage, OutputMessage>,
        storage: Arc<BlockStore>,
    ) -> anyhow::Result<Self> { 
        let peer_states = PeerStates::new(self, storage.clone(), pipe.send);
        let result = scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(peer_states.run_block_fetcher(ctx));
            while let Ok(input_message) = pipe.recv.recv(ctx).await {
                match input_message {
                    InputMessage::Network(SyncBlocksRequest::UpdatePeerSyncState {
                        peer,
                        state,
                        response,
                    }) => {
                        peer_states.update(&peer, state);
                        response.send(()).ok();
                    }
                    InputMessage::Network(SyncBlocksRequest::GetBlock { block_number, response }) => {
                        response.send(storage.block(ctx,block_number).await.map_ok(
                            |b|b.ok_or(GetBlockError::NotSynced)
                        ));
                    }
                }
            }
        }).await;

        // Since we clearly type cancellation errors, it's easier propagate them up to this entry point,
        // rather than catching in the constituent tasks.
        result.or_else(|err| match err {
            ctx::Error::Canceled(_) => Ok(()), // Cancellation is not propagated as an error
            ctx::Error::Internal(err) => Err(err),
        })
    }
}
