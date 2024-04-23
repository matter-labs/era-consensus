//! Gossip network is a sparse graph of connections between nodes (not necessarily validators).
//! It will be used for:
//! * discovery of validators (consensus network will be bootstrapped from data received over
//!   the gossip network).
//! * broadcasting the finalized blocks
//! * P2P block synchronization (for non-validators)
//!
//! Gossip network consists of
//! * static connections (explicitly declared in configs of both ends of the connection).
//! * dynamic connections (additional randomized connections which are established to improve
//!   the throughput of the network).
//! Static connections constitute a rigid "backbone" of the gossip network, which is insensitive to
//! eclipse attack. Dynamic connections are supposed to improve the properties of the gossip
//! network graph (minimize its diameter, increase connectedness).
use crate::{gossip::ValidatorAddrsWatch, io, pool::PoolWatch, Config};
use std::sync::{atomic::AtomicUsize, Arc};
pub(crate) use validator_addrs::*;
use zksync_concurrency::{sync, ctx::channel, scope, ctx};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::BlockStore;

mod handshake;
mod runner;
mod get_block;
#[cfg(test)]
mod tests;
mod validator_addrs;

/// Gossip network state.
pub(crate) struct Network {
    /// Gossip network configuration.
    pub(crate) cfg: Config,
    /// Currently open inbound connections.
    pub(crate) inbound: PoolWatch<node::PublicKey, ()>,
    /// Currently open outbound connections.
    pub(crate) outbound: PoolWatch<node::PublicKey, ()>,
    /// Current state of knowledge about validators' endpoints.
    pub(crate) validator_addrs: ValidatorAddrsWatch,
    /// Block store to serve `get_block` requests from.
    pub(crate) block_store: Arc<BlockStore>,
    /// Output pipe of the network actor.
    pub(crate) sender: channel::UnboundedSender<io::OutputMessage>,
    /// Queue of `get_block` calls to peers.
    pub(crate) get_block_queue: get_block::Queue,
    /// Next block number to finalize.
    pub(crate) global_next: sync::watch::Sender<validator::BlockNumber>,
    /// TESTONLY: how many time push_validator_addrs rpc was called by the peers.
    pub(crate) push_validator_addrs_calls: AtomicUsize,
}

impl Network {
    /// Constructs a new State.
    pub(crate) fn new(
        cfg: Config,
        block_store: Arc<BlockStore>,
        sender: channel::UnboundedSender<io::OutputMessage>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sender,
            inbound: PoolWatch::new(
                cfg.gossip.static_inbound.clone(),
                cfg.gossip.dynamic_inbound_limit,
            ),
            outbound: PoolWatch::new(cfg.gossip.static_outbound.keys().cloned().collect(), 0),
            validator_addrs: ValidatorAddrsWatch::default(),
            cfg,
            get_block_queue: get_block::Queue::default(),
            global_next: sync::watch::channel(block_store.queued().next()).0,
            block_store,
            push_validator_addrs_calls: 0.into(),
        })
    }

    /// Genesis.
    pub(crate) fn genesis(&self) -> &validator::Genesis {
        self.block_store.genesis()
    }

    /// Task fetching blocks from peers which are not present in storage.
    pub(crate) async fn run_block_fetcher(&self, ctx: &ctx::Ctx) {
        const MAX_CONCURRENT_FETCHES: usize = 20;
        let sem = sync::Semaphore::new(MAX_CONCURRENT_FETCHES);
        let _ : ctx::OrCanceled<()> = scope::run!(ctx, |ctx, s| async {
            let mut next = self.block_store.queued().next();
            let global_next = &mut self.global_next.subscribe();
            loop {
                sync::wait_for(ctx, global_next, |g| g >= &next).await?;
                let permit = sync::acquire(ctx, &sem).await?;
                let number = ctx::NoCopy(next);
                next = next+1;
                // Fetch a block asynchronously.
                s.spawn(async {
                    let _permit = permit;
                    let number = number.into();
                    let _ : ctx::OrCanceled<_> = scope::run!(ctx, |ctx, s| async {
                        s.spawn_bg(async {
                            // Retry until fetched.
                            while ctx.is_active() {
                                let res = async {
                                    let block = self.get_block_queue.call(ctx, number).await?;
                                    self.block_store.queue_block(ctx, block).await?;
                                    tracing::info!("fetched block {number}");
                                    ctx::Result::Ok(())
                                }.await;
                                match res {
                                    Ok(()) => return Ok(()),
                                    Err(ctx::Error::Canceled(_)) => {},
                                    Err(ctx::Error::Internal(err)) => tracing::info!(%number, "get_block(): {err:#}"),
                                }
                            }
                            Err(ctx::Canceled)
                        });
                        // Cancel fetching as soon as block is queued for storage.
                        self.block_store.wait_until_queued(ctx, number).await
                    })
                    .await;
                    // Wait until the block is actually persisted, so that the amount of blocks
                    // stored in memory is bounded.
                    self.block_store.wait_until_persisted(ctx, number).await
                });
            }
        })
        .await;
    }
}
