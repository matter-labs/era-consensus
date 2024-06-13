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
use self::batch_votes::BatchVotesWatch;
use crate::{gossip::ValidatorAddrsWatch, io, pool::PoolWatch, Config};
use anyhow::Context as _;
use im::HashMap;
use std::sync::{atomic::AtomicUsize, Arc};
pub(crate) use validator_addrs::*;
use zksync_concurrency::{ctx, ctx::channel, scope, sync};
use zksync_consensus_roles::{attester, node, validator};
use zksync_consensus_storage::BlockStore;

mod batch_votes;
mod fetch;
mod handshake;
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
    pub(crate) inbound: PoolWatch<node::PublicKey, ()>,
    /// Currently open outbound connections.
    pub(crate) outbound: PoolWatch<node::PublicKey, ()>,
    /// Current state of knowledge about validators' endpoints.
    pub(crate) validator_addrs: ValidatorAddrsWatch,
    /// Current state of knowledge about batch votes.
    pub(crate) batch_votes: BatchVotesWatch,
    /// Block store to serve `get_block` requests from.
    pub(crate) block_store: Arc<BlockStore>,
    /// Output pipe of the network actor.
    pub(crate) sender: channel::UnboundedSender<io::OutputMessage>,
    /// Queue of block fetching requests.
    ///
    /// These are blocks that this node wants to request from remote peers via RPC.
    pub(crate) fetch_queue: fetch::Queue,
    /// Last viewed QC.
    pub(crate) last_viewed_qc: Option<attester::BatchQC>,
    /// L1 batch qc.
    pub(crate) batch_qc: HashMap<attester::BatchNumber, attester::BatchQC>,
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
            batch_votes: BatchVotesWatch::default(),
            batch_qc: HashMap::new(),
            last_viewed_qc: None,
            cfg,
            fetch_queue: fetch::Queue::default(),
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
                        s.spawn_bg(self.fetch_queue.request(ctx, number));
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

    /// Task that keeps hearing about new votes and updates the L1 batch qc.
    /// It will propagate the QC if there's enough votes.
    pub(crate) async fn update_batch_qc(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        // TODO This is not a good way to do this, we shouldn't be verifying the QC every time
        // Can we get only the latest votes?
        let attesters = self.genesis().attesters.as_ref().context("attesters")?;
        loop {
            let mut sub = self.batch_votes.subscribe();
            let votes = sync::changed(ctx, &mut sub)
                .await
                .context("batch votes")?
                .clone();

            // Check next QC to collect votes for.
            let new_qc = self
                .last_viewed_qc
                .clone()
                .map(|qc| {
                    attester::BatchQC::new(attester::Batch {
                        number: qc.message.number.next(),
                    })
                })
                .unwrap_or_else(|| {
                    attester::BatchQC::new(attester::Batch {
                        number: attester::BatchNumber(0),
                    })
                })
                .context("new qc")?;

            // Check votes for the correct QC.
            for (_, sig) in votes.0 {
                if self
                    .batch_qc
                    .clone()
                    .entry(new_qc.message.number.clone())
                    .or_insert_with(|| attester::BatchQC::new(new_qc.message.clone()).expect("qc"))
                    .add(&sig, self.genesis())
                    .is_err()
                {
                    // TODO: Should we ban the peer somehow?
                    continue;
                }
            }

            let weight = attesters.weight_of_keys(
                self.batch_qc
                    .get(&new_qc.message.number)
                    .context("last qc")?
                    .signatures
                    .keys(),
            );

            if weight < attesters.threshold() {
                return Ok(());
            };

            // If we have enough weight, we can update the last viewed QC and propagate it.
        }
    }
}
