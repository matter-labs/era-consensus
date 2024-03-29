//! # Sync Blocks Actor
//!
//! This crate contains an actor implementing block syncing among nodes, which is tied to the gossip
//! network RPCs.
use crate::io::{InputMessage, OutputMessage};
use std::sync::Arc;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_network::io::SyncBlocksRequest;
use zksync_consensus_storage::BlockStore;
use zksync_consensus_utils::pipe::ActorPipe;

mod config;
pub mod io;
mod peers;
#[cfg(test)]
mod tests;

pub use crate::config::Config;
use crate::peers::PeerStates;

impl Config {
    /// Runs the sync_blocks actor.
    pub async fn run(
        self,
        ctx: &ctx::Ctx,
        mut pipe: ActorPipe<InputMessage, OutputMessage>,
        storage: Arc<BlockStore>,
    ) -> anyhow::Result<()> {
        let peer_states = PeerStates::new(self, storage.clone(), pipe.send);
        let result: ctx::Result<()> = scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(async { peer_states.run_block_fetcher(ctx).await });
            loop {
                match pipe.recv.recv(ctx).await? {
                    InputMessage::Network(SyncBlocksRequest::UpdatePeerSyncState {
                        peer,
                        state,
                        response,
                    }) => {
                        let res = peer_states.update(&peer, state);
                        if let Err(err) = res {
                            tracing::info!(%err, ?peer, "peer_states.update()");
                        }
                        response.send(()).ok();
                    }
                }
            }
        })
        .await;

        // Since we clearly type cancellation errors, it's easier propagate them up to this entry point,
        // rather than catching in the constituent tasks.
        result.or_else(|err| match err {
            ctx::Error::Canceled(_) => Ok(()), // Cancellation is not propagated as an error
            ctx::Error::Internal(err) => Err(err),
        })
    }
}
