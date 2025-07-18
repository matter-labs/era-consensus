use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use anyhow::Context as _;
use tracing::Instrument;
use zksync_concurrency::{ctx, error::Wrap as _, scope, sync, time};
use zksync_consensus_roles::validator::{self, Block};

use crate::{
    block_store::BlockStore,
    metrics::{self, BLOCK_STORE},
    BlockStoreState, EngineInterface, Last, Transaction,
};

/// A wrapper around a EngineInterface which adds caching blocks in-memory
/// and other useful utilities.
#[derive(Debug)]
pub struct EngineManager {
    // Anything that implements EngineInterface.
    interface: Box<dyn EngineInterface>,
    // The genesis specification of the chain.
    genesis: validator::Genesis,
    // A watch channel over the current block store state.
    block_store: sync::watch::Sender<BlockStore>,
    // A map of epoch number to validator schedule with lifetime information.
    epoch_schedule: sync::watch::Sender<BTreeMap<validator::EpochNumber, ScheduleWithLifetime>>,
    // The interval at which we fetch the pending validator schedule.
    fetch_schedule_interval: time::Duration,
    // A channel to add transactions to be gossiped to the network. We assume that the
    // transactions are already verified. So this is meant to be used by the mempool in
    // the execution layer to send new transactions to the network.
    tx_pool: sync::broadcast::Sender<Transaction>,
}

impl EngineManager {
    /// Constructs a EngineManager.
    /// EngineManager takes ownership of the passed EngineInterface,
    /// i.e. caller should modify the underlying execution layer and
    /// persistent storage ONLY through the constructed EngineManager.
    pub async fn new(
        ctx: &ctx::Ctx,
        interface: Box<dyn EngineInterface>,
        fetch_schedule_interval: time::Duration,
    ) -> ctx::Result<(Arc<Self>, EngineManagerRunner)> {
        // Get the genesis.
        let genesis = interface.genesis(ctx).await.wrap("interface.genesis()")?;

        // Check if the genesis is valid.
        if genesis.protocol_version.0 == 1 && genesis.validators_schedule.is_none() {
            return Err(anyhow::format_err!(
                "genesis is invalid: protocol version is 1 but validators schedule is not set"
            )
            .into());
        }

        // Build the epoch schedule. If the genesis is using a static schedule, we only need to insert
        // the genesis validator schedule in the epoch schedule.
        let mut epoch_schedule = BTreeMap::new();
        if let Some(schedule) = &genesis.validators_schedule {
            epoch_schedule.insert(
                validator::EpochNumber(0),
                ScheduleWithLifetime {
                    schedule: schedule.clone(),
                    activation_block: genesis.first_block,
                    expiration_block: None,
                },
            );
        }

        // Get the persisted state.
        let persisted = interface.persisted().borrow().clone();
        persisted.verify().context("state.verify()")?;

        let this = Arc::new(Self {
            block_store: sync::watch::channel(BlockStore {
                queued: persisted.clone(),
                persisted,
                cache: VecDeque::new(),
            })
            .0,
            genesis,
            interface,
            epoch_schedule: sync::watch::channel(epoch_schedule).0,
            fetch_schedule_interval,
            // We make the tx pool bounded to 1000 transactions. This is a reasonable limit
            // as we expect this channel to be empty most of the time.
            tx_pool: sync::broadcast::channel(1000).0,
        });

        Ok((this.clone(), EngineManagerRunner(this)))
    }

    /// Hash of the genesis specification for this block store.
    pub fn genesis_hash(&self) -> validator::GenesisHash {
        self.genesis.hash()
    }

    /// Chain ID of the genesis specification for this block store.
    pub fn chain_id(&self) -> validator::ChainId {
        self.genesis.chain_id
    }

    /// Fork number of the genesis specification for this block store.
    pub fn fork_number(&self) -> validator::ForkNumber {
        self.genesis.fork_number
    }

    /// Protocol version of the genesis specification for this block store.
    pub fn protocol_version(&self) -> validator::ProtocolVersion {
        self.genesis.protocol_version
    }

    /// First block of the genesis specification for this block store.
    pub fn first_block(&self) -> validator::BlockNumber {
        self.genesis.first_block
    }

    /// Returns true if there is a static validator schedule in the genesis.
    pub fn has_static_schedule(&self) -> bool {
        self.genesis.validators_schedule.is_some()
    }

    /// Returns the validator schedule, with its activation and expiration block numbers,
    /// for the given epoch number.
    pub fn validator_schedule(
        &self,
        epoch: validator::EpochNumber,
    ) -> Option<ScheduleWithLifetime> {
        self.epoch_schedule.borrow().get(&epoch).cloned()
    }

    /// Returns the block number of the head of the chain. We might not have this block in store
    /// if it was pruned or we just started from a snapshot of the state without any blocks.
    pub fn head(&self) -> validator::BlockNumber {
        self.block_store.borrow().persisted.head()
    }

    /// Available blocks (in-memory & persisted).
    pub fn queued(&self) -> BlockStoreState {
        self.block_store.borrow().queued.clone()
    }

    /// Persisted blocks.
    pub fn persisted(&self) -> BlockStoreState {
        self.block_store.borrow().persisted.clone()
    }

    /// Fetches a block (from queue or persistent storage).
    pub async fn get_block(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<Option<Block>> {
        {
            let block_store = self.block_store.borrow();
            if !block_store.queued.contains(number) {
                return Ok(None);
            }
            if let Some(block) = block_store.block(number) {
                return Ok(Some(block));
            }
        }

        let t = metrics::ENGINE_INTERFACE.get_block_latency.start();
        let block = self
            .interface
            .get_block(ctx, number)
            .await
            .wrap("persistent.block()")?;
        t.observe();

        Ok(Some(block))
    }

    /// Append block to a queue to be persisted eventually.
    /// Since persisting a block may take a significant amount of time,
    /// BlockStore contains a queue of blocks waiting to be persisted.
    /// `queue_block()` adds a block to the queue as soon as all intermediate
    /// blocks are queued_state as well. Queue is unbounded, so it is caller's
    /// responsibility to manage the queue size.
    pub async fn queue_block(&self, ctx: &ctx::Ctx, block: Block) -> ctx::Result<()> {
        // Verify the block.
        match &block {
            Block::PreGenesis(b) => {
                if b.number >= self.genesis.first_block {
                    return Err(anyhow::format_err!(
                        "external justification is allowed only for pre-genesis blocks"
                    )
                    .into());
                }
                let t = metrics::ENGINE_INTERFACE
                    .verify_pregenesis_block_latency
                    .start();
                self.interface
                    .verify_pregenesis_block(ctx, b)
                    .await
                    .context("verify_pregenesis_block()")?;
                t.observe();
            }
            Block::FinalV2(b) => {
                let epoch = b.epoch();
                if let Some(schedule_with_lifetime) = self.validator_schedule(epoch) {
                    b.verify(self.genesis.hash(), epoch, &schedule_with_lifetime.schedule)
                        .context("block_v2.verify()")?;
                } else {
                    return Err(anyhow::format_err!(
                        "cannot verify block v2: epoch schedule is not available"
                    )
                    .into());
                }
            }
        }

        sync::wait_for(ctx, &mut self.block_store.subscribe(), |block_store| {
            block_store.queued.next() >= block.number()
        })
        .await?;

        self.block_store
            .send_if_modified(|block_store| block_store.try_push(block));

        Ok(())
    }

    /// Waits until the queued blocks range is different than `old`.
    pub async fn wait_for_queued_change(
        &self,
        ctx: &ctx::Ctx,
        old: &BlockStoreState,
    ) -> ctx::OrCanceled<BlockStoreState> {
        sync::wait_for_some(ctx, &mut self.block_store.subscribe(), |block_store| {
            if &block_store.queued == old {
                return None;
            }
            Some(block_store.queued.clone())
        })
        .await
    }

    /// Waits until the given block is queued (in memory, or persisted).
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    #[tracing::instrument(skip_all, fields(l2_block = %number))]
    pub async fn wait_until_queued(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<BlockStoreState> {
        Ok(
            sync::wait_for(ctx, &mut self.block_store.subscribe(), |block_store| {
                number < block_store.queued.next()
            })
            .await?
            .queued
            .clone(),
        )
    }

    /// Waits until the given block is stored persistently.
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    #[tracing::instrument(skip_all, fields(l2_block = %number))]
    pub async fn wait_until_persisted(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<BlockStoreState> {
        Ok(sync::wait_for(
            ctx,
            &mut self.interface.persisted(),
            |persisted: &BlockStoreState| number < persisted.next(),
        )
        .await?
        .clone())
    }

    /// Waits until the epoch schedule is populated. Returns the number of the first epoch stored.
    pub async fn wait_until_epoch_schedule_populated(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::OrCanceled<validator::EpochNumber> {
        sync::wait_for(
            ctx,
            &mut self.epoch_schedule.subscribe(),
            |epoch_schedule| epoch_schedule.keys().next().is_some(),
        )
        .await?;

        Ok(self.epoch_schedule.borrow().keys().next().cloned().unwrap()) // unwrap is safe because we know that the epoch schedule is populated
    }

    /// Waits until the validator schedule for the given epoch is available.
    pub async fn wait_for_validator_schedule(
        &self,
        ctx: &ctx::Ctx,
        epoch: validator::EpochNumber,
    ) -> ctx::OrCanceled<ScheduleWithLifetime> {
        sync::wait_for(
            ctx,
            &mut self.epoch_schedule.subscribe(),
            |epoch_schedule| epoch_schedule.get(&epoch).is_some(),
        )
        .await?;

        Ok(self.validator_schedule(epoch).unwrap()) // unwrap is safe because we know that the schedule is available
    }

    /// Waits until the validator schedule for the given epoch has an expiration block.
    pub async fn wait_for_validator_schedule_expiration(
        &self,
        ctx: &ctx::Ctx,
        epoch: validator::EpochNumber,
    ) -> ctx::OrCanceled<validator::BlockNumber> {
        sync::wait_for(
            ctx,
            &mut self.epoch_schedule.subscribe(),
            |epoch_schedule| {
                epoch_schedule
                    .get(&epoch)
                    .map(|schedule| schedule.expiration_block.is_some())
                    .unwrap_or(false)
            },
        )
        .await?;

        Ok(self
            .epoch_schedule
            .borrow()
            .get(&epoch)
            .unwrap() // unwrap is safe because we know that the schedule for the given epoch is available
            .expiration_block
            .unwrap()) // unwrap is safe because we know that the schedule has an expiration block
    }

    /// Verifies a payload.
    pub async fn verify_payload(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
        epoch: validator::EpochNumber,
        payload: &validator::Payload,
    ) -> ctx::Result<()> {
        // Check that this block belongs to the given epoch, otherwise return an error.
        // This is a sanity check to ensure that the payload is being verified for the correct epoch.
        if self.epoch_for_block(number) != Some(epoch) {
            return Err(
                anyhow::format_err!("block {} does not belong to epoch {}", number, epoch).into(),
            );
        }

        let t = metrics::ENGINE_INTERFACE.verify_payload_latency.start();
        self.interface
            .verify_payload(ctx, number, payload)
            .await
            .context("verify_payload()")?;
        t.observe();
        Ok(())
    }

    /// Proposes a payload.
    pub async fn propose_payload(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload> {
        let t = metrics::ENGINE_INTERFACE.propose_payload_latency.start();
        let payload = self
            .interface
            .propose_payload(ctx, number)
            .await
            .wrap("propose_payload()")?;
        t.observe();
        Ok(payload)
    }

    /// Gets the replica state.
    pub async fn get_state(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::ReplicaState> {
        let t = metrics::ENGINE_INTERFACE.get_state_latency.start();
        let state = self.interface.get_state(ctx).await.wrap("get_state()")?;
        t.observe();
        Ok(state)
    }

    /// Sets the replica state.
    pub async fn set_state(
        &self,
        ctx: &ctx::Ctx,
        state: &validator::ReplicaState,
    ) -> ctx::Result<()> {
        let t = metrics::ENGINE_INTERFACE.set_state_latency.start();
        self.interface
            .set_state(ctx, state)
            .await
            .context("set_state()")?;
        t.observe();
        Ok(())
    }

    /// Pushes a transaction to the mempool in the execution layer. If it's a valid transaction
    /// and hasn't been added before, it will be added to the mempool now and return true.
    /// Otherwise, it will return false.
    pub async fn push_tx(&self, ctx: &ctx::Ctx, tx: Transaction) -> ctx::Result<bool> {
        let t = metrics::ENGINE_INTERFACE.push_tx_latency.start();
        let accepted = self.interface.push_tx(ctx, tx).await.wrap("push_tx()")?;
        t.observe();
        Ok(accepted)
    }

    /// Returns a Sender handle to the tx pool. It can be used to send transactions to be gossiped
    /// to the network. We assume that the transactions are already verified. So this is meant to be
    /// used by the mempool in the execution layer to send new transactions to the network.
    /// Note that this is bounded, as documented in the tokio broadcast channel documentation, so
    /// transactions might get dropped if the channel is full.
    pub fn tx_pool_sender(&self) -> sync::broadcast::Sender<Transaction> {
        self.tx_pool.clone()
    }

    /// Returns a Receiver handle to the tx pool. It is used to receive transactions to be gossiped
    /// to the network.
    pub fn tx_pool_receiver(&self) -> sync::broadcast::Receiver<Transaction> {
        self.tx_pool.subscribe()
    }

    /// Returns the epoch number for the given block number, if it exists.
    fn epoch_for_block(&self, number: validator::BlockNumber) -> Option<validator::EpochNumber> {
        self.epoch_schedule
            .borrow()
            .iter()
            .find(|(_, schedule)| {
                schedule.activation_block <= number
                    && (schedule.expiration_block.is_none()
                        || schedule.expiration_block.unwrap() >= number)
            })
            .map(|(epoch, _)| *epoch)
    }

    fn scrape_metrics(&self) -> metrics::BlockStore {
        let m = metrics::BlockStore::default();
        let block_store = self.block_store.borrow();
        m.next_queued_block.set(block_store.queued.next().0);
        m.next_persisted_block.set(block_store.persisted.next().0);
        m.cache_size.set(block_store.cache.len() as u64);
        m
    }
}

/// Runner of the EngineManager background tasks.
#[must_use]
#[derive(Debug, Clone)]
pub struct EngineManagerRunner(Arc<EngineManager>);

impl EngineManagerRunner {
    /// Runs the background tasks of the EngineManager.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let store_ref = Arc::downgrade(&self.0);
        let _ = BLOCK_STORE.before_scrape(move || Some(store_ref.upgrade()?.scrape_metrics()));

        let res = scope::run!(ctx, |ctx, s| async {
            // Task watching the persisted state.
            s.spawn::<()>(async {
                let mut persisted = self.0.interface.persisted();
                persisted.mark_changed();
                loop {
                    async {
                        let new = sync::changed::<BlockStoreState>(ctx, &mut persisted)
                            .instrument(tracing::trace_span!("wait_for_block_store_change"))
                            .await?
                            .clone();
                        sync::try_send_modify(&self.0.block_store, |block_store| {
                            block_store.update_persisted(new)
                        })?;

                        ctx::Ok(())
                    }
                    .instrument(tracing::trace_span!("watch_persistent_state_iteration"))
                    .await?;
                }
            });

            // Task queueing blocks to be persisted.
            s.spawn::<()>(async {
                let block_store = &mut self.0.block_store.subscribe();
                let mut queue_next = validator::BlockNumber(0);
                loop {
                    async {
                        let block = sync::wait_for_some(ctx, block_store, |block_store| {
                            block_store.block(queue_next.max(block_store.persisted.next()))
                        })
                        .instrument(tracing::trace_span!("wait_for_next_block"))
                        .await?;
                        queue_next = block.number().next();

                        let t = metrics::ENGINE_INTERFACE.queue_next_block_latency.start();
                        self.0.interface.queue_next_block(ctx, block).await?;
                        t.observe();

                        ctx::Ok(())
                    }
                    .instrument(tracing::trace_span!("queue_persist_block_iteration"))
                    .await?;
                }
            });

            // Task updating the validator schedule.
            // If we are not using a static schedule, we need to update the validator schedule for each epoch.
            if !self.0.has_static_schedule() {
                s.spawn::<()>(async {
                    // Wait until we persisted all pre-genesis blocks.
                    // This is necessary because we only need the validator schedule after genesis.
                    if let Some(last_pre_genesis) = self.0.genesis.first_block.prev() {
                        self.0.wait_until_persisted(ctx, last_pre_genesis).await?;
                    }

                    // At this point we should not be able to sync any more blocks as we are missing
                    // the validator schedule for the next blocks.

                    // Get the current block and epoch number.
                    let (mut head, mut cur_epoch) = match self.0.persisted().last {
                        Some(Last::FinalV2(qc)) => (qc.header().number, qc.message.view.epoch),
                        Some(Last::PreGenesis(n)) => (n, validator::EpochNumber(0)),
                        None => (self.0.genesis.first_block, validator::EpochNumber(0)),
                    };

                    // Get the current validator schedule and insert it into the epoch schedule.
                    let current_schedule = self
                        .0
                        .interface
                        .get_validator_schedule(ctx, head)
                        .await
                        .wrap("interface.get_validator_schedule()")?;

                    self.0.epoch_schedule.send_modify(|epoch_schedule| {
                        epoch_schedule.insert(
                            cur_epoch,
                            ScheduleWithLifetime {
                                schedule: current_schedule.0,
                                activation_block: current_schedule.1,
                                expiration_block: None,
                            },
                        );
                    });

                    // Start a loop trying to fetch the pending validator schedules for the next epochs.
                    loop {
                        async {
                            head = self.0.persisted().head();

                            // Check if head is after the activation block of the LAST epoch in epoch_schedule
                            let last_epoch_activation = self
                                .0
                                .epoch_schedule
                                .borrow()
                                .iter()
                                .last()
                                .unwrap() // unwrap is safe because we know that there is at least one epoch
                                .1
                                .activation_block;

                            // Only try to get the pending schedule if we're past the activation block of the last epoch.
                            // Otherwise, it's guaranteed that there's no new pending schedule.
                            if head > last_epoch_activation {
                                // Now we can check for a pending schedule.
                                if let Some(pending_schedule) = self
                                    .0
                                    .interface
                                    .get_pending_validator_schedule(ctx, head)
                                    .await
                                    .wrap("interface.get_pending_validator_schedule()")?
                                {
                                    // Update the epoch schedule.
                                    self.0.epoch_schedule.send_modify(|epoch_schedule| {
                                        // Insert the new validator schedule.
                                        epoch_schedule.insert(
                                            cur_epoch.next(),
                                            ScheduleWithLifetime {
                                                schedule: pending_schedule.0,
                                                activation_block: pending_schedule.1,
                                                expiration_block: None,
                                            },
                                        );

                                        // Update the previous validator schedule expiration block.
                                        if let Some(prev_epoch_entry) =
                                            epoch_schedule.get_mut(&cur_epoch)
                                        {
                                            prev_epoch_entry.expiration_block =
                                                Some(pending_schedule.1.prev().unwrap());
                                            // unwrap is safe because we know that there was a previous epoch
                                        }

                                        // See if we can prune the oldest validator schedule.
                                        if let Some(schedule) = epoch_schedule.iter().nth(2) {
                                            if schedule.1.activation_block < head {
                                                epoch_schedule.pop_first();
                                            }
                                        }
                                    });

                                    cur_epoch = cur_epoch.next();
                                }
                            }

                            ctx::Ok(())
                        }
                        .instrument(tracing::trace_span!("update_validator_schedule_iteration"))
                        .await?;

                        // Epochs should be at least minutes apart so that validators have time to
                        // establish network connections. So we don't need to check for new epochs too often.
                        ctx.sleep(self.0.fetch_schedule_interval).await?;
                    }
                });
            }

            ctx::Ok(())
        })
        .await;

        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}

/// A validator schedule with its activation and expiration block numbers.
#[derive(Clone, Debug)]
pub struct ScheduleWithLifetime {
    /// The validator schedule.
    pub schedule: validator::Schedule,
    /// The block number at which the schedule becomes active. Note that the schedule is already
    /// active at the activation block.
    pub activation_block: validator::BlockNumber,
    /// The block number at which the schedule expires. It might not be present
    /// if the schedule is static or the the next schedule is not yet known.
    /// Note that the schedule is still active at the expiration block.
    pub expiration_block: Option<validator::BlockNumber>,
}
