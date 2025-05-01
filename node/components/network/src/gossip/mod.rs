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
//!
//! Static connections constitute a rigid "backbone" of the gossip network, which is insensitive to
//! eclipse attack. Dynamic connections are supposed to improve the properties of the gossip
//! network graph (minimize its diameter, increase connectedness).
use std::sync::{atomic::AtomicUsize, Arc};

use anyhow::Context as _;
use fetch::RequestItem;
use tracing::Instrument;
pub(crate) use validator_addrs::*;
use zksync_concurrency::{ctx, scope, sync};
use zksync_consensus_engine::EngineManager;
use zksync_consensus_roles::{node, validator};

use crate::{gossip::ValidatorAddrsWatch, io, pool::PoolWatch, Config, MeteredStreamStats};

mod fetch;
mod handshake;
pub mod loadtest;
mod runner;
#[cfg(test)]
mod testonly;
#[cfg(test)]
mod tests;
mod validator_addrs;

/// Info about a gossip connection.
#[derive(Debug)]
pub(crate) struct Connection {
    /// Peer's public key.
    pub(crate) key: node::PublicKey,
    /// Build version of peer's binary (not verified).
    pub(crate) build_version: Option<semver::Version>,
    /// TCP connection stats.
    pub(crate) stats: Arc<MeteredStreamStats>,
}

/// Gossip network state.
pub(crate) struct Network {
    /// The epoch number for this instance of the network component.
    /// If None, the network component will only fetch pre-genesis blocks.
    pub(crate) epoch_number: Option<validator::EpochNumber>,
    /// Gossip network configuration.
    pub(crate) cfg: Config,
    /// Currently open inbound connections.
    pub(crate) inbound: PoolWatch<node::PublicKey, Arc<Connection>>,
    /// Currently open outbound connections.
    pub(crate) outbound: PoolWatch<node::PublicKey, Arc<Connection>>,
    /// Current state of knowledge about validators' endpoints.
    pub(crate) validator_addrs: ValidatorAddrsWatch,
    /// Engine manager to serve `get_block` requests from.
    pub(crate) engine_manager: Arc<EngineManager>,
    /// Sender of the channel to the consensus component.
    pub(crate) consensus_sender: sync::prunable_mpsc::Sender<io::ConsensusReq>,
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
        engine_manager: Arc<EngineManager>,
        epoch_number: Option<validator::EpochNumber>,
        consensus_sender: sync::prunable_mpsc::Sender<io::ConsensusReq>,
    ) -> Arc<Self> {
        Arc::new(Self {
            epoch_number,
            consensus_sender,
            inbound: PoolWatch::new(
                cfg.gossip.static_inbound.clone(),
                cfg.gossip.dynamic_inbound_limit,
            ),
            outbound: PoolWatch::new(cfg.gossip.static_outbound.keys().cloned().collect(), 0),
            validator_addrs: ValidatorAddrsWatch::default(),
            cfg,
            fetch_queue: fetch::Queue::default(),
            engine_manager,
            push_validator_addrs_calls: 0.into(),
        })
    }

    /// Genesis hash.
    pub(crate) fn genesis_hash(&self) -> validator::GenesisHash {
        self.engine_manager.genesis_hash()
    }

    /// Genesis first block.
    pub(crate) fn first_block(&self) -> validator::BlockNumber {
        self.engine_manager.first_block()
    }

    /// Validator schedule for this epoch. If None, we are only fetching pre-genesis blocks.
    pub(crate) fn validator_schedule(&self) -> anyhow::Result<Option<validator::Schedule>> {
        if let Some(epoch_number) = self.epoch_number {
            Ok(Some(
                self.engine_manager
                    .validator_schedule(epoch_number)
                    .context(format!(
                        "Network instance was started for epoch {} but there's no \
                                 corresponding validator schedule.",
                        epoch_number,
                    ))?
                    .schedule,
            ))
        } else {
            Ok(None)
        }
    }

    /// Task fetching blocks from peers which are not present in storage.
    pub(crate) async fn run_block_fetcher(&self, ctx: &ctx::Ctx) {
        let sem = sync::Semaphore::new(self.cfg.max_block_queue_size);
        let _: ctx::OrCanceled<()> = scope::run!(ctx, |ctx, s| async {
            let mut next = self.engine_manager.queued().next();
            loop {
                let permit = sync::acquire(ctx, &sem).await?;
                let number = ctx::NoCopy(next);
                next = next + 1;
                // Fetch a block asynchronously.
                s.spawn(
                    async {
                        let _permit = permit;
                        let number = number.into();
                        let _: ctx::OrCanceled<()> = scope::run!(ctx, |ctx, s| async {
                            s.spawn_bg(
                                self.fetch_queue
                                    .request(ctx, RequestItem::Block(number))
                                    .instrument(tracing::info_span!("fetch_block_request")),
                            );
                            // Cancel fetching as soon as block is queued for storage.
                            self.engine_manager.wait_until_queued(ctx, number).await?;
                            Err(ctx::Canceled)
                        })
                        .instrument(tracing::info_span!("wait_for_block_to_queue"))
                        .await;
                        // Wait until the block is actually persisted, so that the amount of blocks
                        // stored in memory is bounded.
                        self.engine_manager.wait_until_persisted(ctx, number).await
                    }
                    .instrument(tracing::info_span!("fetch_block_from_peer", l2_block = %next)),
                );
            }
        })
        .await;
    }
}
