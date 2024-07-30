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
pub use self::attestation_status::{
    AttestationStatusClient, AttestationStatusReceiver, LocalAttestationStatus,
};
pub use self::batch_votes::BatchVotesPublisher;
use self::batch_votes::BatchVotesWatch;
use crate::{gossip::ValidatorAddrsWatch, io, pool::PoolWatch, Config, MeteredStreamStats};
use attestation_status::AttestationStatusWatch;
use fetch::RequestItem;
use std::sync::{atomic::AtomicUsize, Arc};
pub(crate) use validator_addrs::*;
use zksync_concurrency::time::Duration;
use zksync_concurrency::{ctx, ctx::channel, error::Wrap as _, scope, sync};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{BatchStore, BlockStore};

mod attestation_status;
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
    /// Shared watch over the current attestation status as indicated by the main node.
    pub(crate) attestation_status: Arc<AttestationStatusWatch>,
    /// Client to use to check the current attestation status on the main node.
    pub(crate) attestation_status_client: Box<dyn AttestationStatusClient>,
}

impl Network {
    /// Constructs a new State.
    pub(crate) fn new(
        cfg: Config,
        block_store: Arc<BlockStore>,
        batch_store: Arc<BatchStore>,
        sender: channel::UnboundedSender<io::OutputMessage>,
        attestation_status_client: Box<dyn AttestationStatusClient>,
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
            attestation_status: Arc::new(AttestationStatusWatch::default()),
            attestation_status_client,
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

    /// Task that reacts to new votes being added and looks for an L1 batch QC.
    /// It persists the certificate once the quorum threshold is passed.
    pub(crate) async fn run_batch_qc_finder(&self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        let Some(attesters) = self.genesis().attesters.as_ref() else {
            tracing::info!("no attesters in genesis, not looking for batch QCs");
            return Ok(());
        };
        let genesis = self.genesis().hash();

        let mut recv_votes = self.batch_votes.subscribe();
        let mut recv_status = self.attestation_status.subscribe();

        let mut prev_batch_number = None;

        loop {
            // Wait until the status indicates that we're ready to sign the next batch.
            // This is not strictly necessary but avoids repeatedly finding the same quorum, or having to skip it until it changes.
            let next_batch_number =
                sync::wait_for_some(ctx, &mut recv_status, |s| match s.next_batch_to_attest {
                    next if next == prev_batch_number => None,
                    next => next,
                })
                .await?;

            // Next time we'll look for something new.
            prev_batch_number = Some(next_batch_number);

            // Get rid of all previous votes. We don't expect this to go backwards without regenesis, which will involve a restart.
            self.batch_votes
                .set_min_batch_number(next_batch_number)
                .await;

            // Now wait until we find the next quorum, whatever it is:
            // * on the main node, if attesters are honest, they will vote on the next batch number and the main node will not see gaps
            // * on external nodes the votes might be affected by changes in the value returned by the API, and there might be gaps
            // What is important, though, is that the batch number does not move backwards while we look for a quorum, because attesters
            // (re)casting earlier votes will go ignored by those fixed on a higher min_batch_number, and gossip will only be attempted once.
            // The possibility of this will be fixed by deterministally picking a start batch number based on fork indicated by genesis.
            let quorum = sync::wait_for_some(ctx, &mut recv_votes, |votes| {
                votes.find_quorum(attesters, &genesis)
            })
            .await?;

            self.batch_store
                .persist_batch_qc(ctx, qc)
                .await
                .wrap("persist_batch_qc")?;
        }
    }

    /// Poll the attestation status and update the watch.
    pub(crate) async fn run_attestation_client(&self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        if self.genesis().attesters.is_none() {
            tracing::info!("no attesters in genesis, not polling the attestation status");
            return Ok(());
        };

        const POLL_INTERVAL: Duration = Duration::seconds(5);

        loop {
            match self
                .attestation_status_client
                .next_batch_to_attest(ctx)
                .await
            {
                Ok(Some(batch_number)) => {
                    self.attestation_status.update(batch_number).await;
                    // We could also update the minimum batch number here, which might
                    // help mitigate the problem of missing a vote if the batch number
                    // happened to decrease. But we decided to fix it at the source,
                    // so the only place that is adjusted is before looking for a QC.
                }
                Ok(None) => tracing::debug!("waiting for attestation status..."),
                Err(error) => tracing::error!(
                    ?error,
                    "failed to poll attestation status, retrying later..."
                ),
            }
            ctx.sleep(POLL_INTERVAL).await?;
        }
    }
}
