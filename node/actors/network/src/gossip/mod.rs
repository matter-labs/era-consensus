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
pub use self::batch_votes::BatchVotesPublisher;
use self::batch_votes::BatchVotesWatch;
use crate::{gossip::ValidatorAddrsWatch, io, pool::PoolWatch, Config, MeteredStreamStats};
use fetch::RequestItem;
use std::sync::{atomic::AtomicUsize, Arc};
pub(crate) use validator_addrs::*;
use zksync_concurrency::{ctx, ctx::channel, error::Wrap as _, scope, sync};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{BatchStore, BlockStore};

mod batch_votes;
mod fetch;
mod handshake;
pub mod loadtest;
mod metrics;
mod runner;
#[cfg(test)]
mod testonly;
#[cfg(test)]
mod tests;
mod validator_addrs;

/// Gossip network state.
pub(crate) struct Network {
    /// Gossip network configuration.
    pub(crate) cfg: Config,
    /// Currently open inbound connections.
    pub(crate) inbound: PoolWatch<node::PublicKey, Arc<MeteredStreamStats>>,
    /// Currently open outbound connections.
    pub(crate) outbound: PoolWatch<node::PublicKey, Arc<MeteredStreamStats>>,
    /// Current state of knowledge about validators' endpoints.
    pub(crate) validator_addrs: ValidatorAddrsWatch,
    /// Current state of knowledge about batch votes.
    pub(crate) batch_votes: Arc<BatchVotesWatch>,
    /// Block store to serve `get_block` requests from.
    pub(crate) block_store: Arc<BlockStore>,
    /// Batch store to serve `get_batch` requests from.
    pub(crate) batch_store: Arc<BatchStore>,
    /// Output pipe of the network actor.
    pub(crate) sender: channel::UnboundedSender<io::OutputMessage>,
    /// Queue of block fetching requests.
    ///
    /// These are blocks that this node wants to request from remote peers via RPC.
    pub(crate) fetch_queue: fetch::Queue,
    /// TESTONLY: how many time push_validator_addrs rpc was called by the peers.
    pub(crate) push_validator_addrs_calls: AtomicUsize,
}

impl Network {
    /// Constructs a new State.
    pub(crate) fn new(
        cfg: Config,
        block_store: Arc<BlockStore>,
        batch_store: Arc<BatchStore>,
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
            batch_votes: Arc::new(BatchVotesWatch::default()),
            cfg,
            fetch_queue: fetch::Queue::default(),
            block_store,
            batch_store,
            push_validator_addrs_calls: 0.into(),
        })
    }

    /// Genesis.
    pub(crate) fn genesis(&self) -> &validator::Genesis {
        self.block_store.genesis()
    }

    /// Task fetching blocks from peers which are not present in storage.
    pub(crate) async fn run_block_fetcher(&self, ctx: &ctx::Ctx) {
        let sem = sync::Semaphore::new(self.cfg.max_block_queue_size);
        let _: ctx::OrCanceled<()> = scope::run!(ctx, |ctx, s| async {
            let mut next = self.block_store.queued().next();
            loop {
                let permit = sync::acquire(ctx, &sem).await?;
                let number = ctx::NoCopy(next);
                next = next + 1;
                // Fetch a block asynchronously.
                s.spawn(async {
                    let _permit = permit;
                    let number = number.into();
                    let _: ctx::OrCanceled<()> = scope::run!(ctx, |ctx, s| async {
                        s.spawn_bg(self.fetch_queue.request(ctx, RequestItem::Block(number)));
                        // Cancel fetching as soon as block is queued for storage.
                        self.block_store.wait_until_queued(ctx, number).await?;
                        Err(ctx::Canceled)
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

    /// Task fetching batches from peers which are not present in storage.
    pub(crate) async fn run_batch_fetcher(&self, ctx: &ctx::Ctx) {
        let sem = sync::Semaphore::new(self.cfg.max_block_queue_size);
        let _: ctx::OrCanceled<()> = scope::run!(ctx, |ctx, s| async {
            let mut next = self.batch_store.queued().next();
            loop {
                let permit = sync::acquire(ctx, &sem).await?;
                let number = ctx::NoCopy(next);
                next = next + 1;
                // Fetch a batch asynchronously.
                s.spawn(async {
                    let _permit = permit;
                    let number = number.into();
                    let _: ctx::OrCanceled<()> = scope::run!(ctx, |ctx, s| async {
                        s.spawn_bg(self.fetch_queue.request(ctx, RequestItem::Batch(number)));
                        // Cancel fetching as soon as batch is queued for storage.
                        self.batch_store.wait_until_queued(ctx, number).await?;
                        Err(ctx::Canceled)
                    })
                    .await;
                    // Wait until the batch is actually persisted, so that the amount of batches
                    // stored in memory is bounded.
                    self.batch_store.wait_until_persisted(ctx, number).await
                });
            }
        })
        .await;
    }

    /// Task that keeps hearing about new votes and looks for an L1 batch qc.
    /// It will propagate the QC if there's enough votes.
    pub(crate) async fn run_batch_qc_finder(&self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        let Some(attesters) = self.genesis().attesters.as_ref() else {
            return Ok(());
        };
        let genesis = self.genesis().hash();
        let mut sub = self.batch_votes.subscribe();
        loop {
            // In the future when we might be gossiping about multiple batches at the same time,
            // we can collect the ones we submitted into a skip list until we see them confirmed
            // on L1 and we can finally increase the minimum as well.
            let quorums = {
                let votes = sync::changed(ctx, &mut sub).await?;
                votes.find_quorums(attesters, &genesis, |_| false)
            };

            for qc in quorums {
                // In the future this should come from confirmations, but for now it's best effort, so we can forget ASAP.
                // TODO: An initial value could be looked up in the database even now.
                let next_batch_number = qc.message.number.next();

                self.batch_store
                    .persist_batch_qc(ctx, qc)
                    .await
                    .wrap("persist_batch_qc")?;

                self.batch_votes
                    .set_min_batch_number(next_batch_number)
                    .await;
            }
        }
    }
}
