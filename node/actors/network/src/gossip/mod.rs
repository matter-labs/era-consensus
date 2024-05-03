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
use im::HashMap;
use std::sync::{atomic::AtomicUsize, Arc};
pub(crate) use validator_addrs::*;
use zksync_concurrency::{ctx, ctx::channel, scope, sync};
use zksync_consensus_roles::{
    attester::{self, BatchNumber, L1BatchQC},
    node, validator,
};
use zksync_consensus_storage::BlockStore;

use self::batch_signatures::L1BatchSignaturesWatch;

mod batch_signatures;
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
    /// Current state of knowledge about batch signatures.
    pub(crate) batch_signatures: L1BatchSignaturesWatch,
    /// Block store to serve `get_block` requests from.
    pub(crate) block_store: Arc<BlockStore>,
    /// Output pipe of the network actor.
    pub(crate) sender: channel::UnboundedSender<io::OutputMessage>,
    /// Queue of block fetching requests.
    pub(crate) fetch_queue: fetch::Queue,
    /// L1 batches.
    // pub(crate) l1_batches: sync::watch::Receiver<Option<L1Batch>>,
    /// Last viewed QC.
    pub(crate) last_viewed_qc: Option<L1BatchQC>,
    /// Attester SecretKey, None if the node is not an attester.
    pub(crate) attester_key: Option<attester::SecretKey>,
    /// L1 batch qc.
    pub(crate) l1_batch_qc: HashMap<BatchNumber, L1BatchQC>,
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
            batch_signatures: L1BatchSignaturesWatch::default(),
            attester_key: cfg.attester_key.clone(),
            l1_batch_qc: HashMap::new(),
            // l1_batches: sync::watch::channel(None).1,
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

    /// Task that keeps hearing about new signatures and updates the L1 batch qc.
    /// It also propagates the QC if there's enough signatures.
    pub(crate) async fn update_batch_qc(&self, ctx: &ctx::Ctx) {
        // FIXME This is not a good way to do this, we shouldn't be verifying the QC every time
        loop {
            let mut sub = self.batch_signatures.subscribe();
            let signatures = sync::changed(ctx, &mut sub).await.unwrap().clone();
            for (_, sig) in signatures.0 {
                self.l1_batch_qc
                    .clone()
                    .entry(sig.msg.number.clone())
                    .or_insert_with(|| L1BatchQC::new(sig.msg.clone(), self.genesis()))
                    .add(&sig, self.genesis());
            }
            // Now we check if we have enough weight to continue.
            if let Some(last_qc) = self.last_viewed_qc.clone() {
                let weight = self.genesis().attesters_committee.weight(
                    &self
                        .l1_batch_qc
                        .get(&last_qc.message.number)
                        .unwrap()
                        .signers,
                );
                if weight > self.genesis().attesters_committee.threshold() {
                    // TODO: Verify and Propagate QC.
                };
            } else if let Some(qc) = self.l1_batch_qc.get(&BatchNumber(0)) {
                let weight = self
                    .genesis()
                    .attesters_committee
                    .weight(&self.l1_batch_qc.get(&qc.message.number).unwrap().signers);
                if weight > self.genesis().attesters_committee.threshold() {
                    // TODO: Verify and Propagate QC.
                };
            }
        }
    }
}
