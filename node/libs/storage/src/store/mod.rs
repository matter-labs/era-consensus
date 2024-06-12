//! Defines storage layer for finalized blocks and batches.
mod metrics;
mod persistent;
mod state;

pub use persistent::PersistentStore;
pub use state::BatchStoreState;
pub use state::BlockStoreState;

use std::{collections::VecDeque, sync::Arc};

use anyhow::Context as _;
use zksync_concurrency::{ctx, error::Wrap, scope, sync};
use zksync_consensus_roles::{attester, validator};

#[derive(Debug)]
struct Inner {
    queued: (BatchStoreState, BlockStoreState),
    persisted: (BatchStoreState, BlockStoreState),
    cache: VecDeque<attester::SyncBatch>,
}

/// A wrapper around a PersistentBatchStore which adds caching blocks in-memory
/// and other useful utilities.
#[derive(Debug)]
pub struct BlockStore {
    inner: sync::watch::Sender<Inner>,
    persistent: Box<dyn PersistentStore>,
    genesis: validator::Genesis,
}

impl Inner {
    /// Minimal number of most recent blocks to keep in memory.
    /// It allows to serve the recent blocks to peers fast, even
    /// if persistent storage reads are slow (like in RocksDB).
    /// `BlockStore` may keep in memory more blocks in case
    /// blocks are queued faster than they are persisted.
    const CACHE_CAPACITY: usize = 10;

    /// Tries to push the next batch to cache.
    /// Noop if provided block is not the expected one.
    /// Returns true iff cache has been modified.
    fn try_push(&mut self, batch: attester::SyncBatch) -> bool {
        if self.queued.0.next() != batch.number {
            return false;
        }
        self.queued.0.last = Some(batch.clone());
        self.cache.push_back(batch);
        self.truncate_cache();
        true
    }

    fn truncate_cache(&mut self) {
        while self.cache.len() > Self::CACHE_CAPACITY
            && self.persisted.0.contains(self.cache[0].number)
        {
            self.cache.pop_front();
        }
    }

    fn batch(&self, n: attester::BatchNumber) -> Option<attester::SyncBatch> {
        // Subtraction is safe, because blocks in cache are
        // stored in increasing order of block number.
        let first = self.cache.front()?;
        self.cache.get((n.0 - first.number.0) as usize).cloned()
    }
}

/// Runner of the BlockStore background tasks.
#[must_use]
#[derive(Debug, Clone)]
pub struct StoreRunner(Arc<BlockStore>);

impl StoreRunner {
    /// Runs the background tasks of the BlockStore.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        #[vise::register]
        static COLLECTOR: vise::Collector<Option<metrics::BlockStore>> = vise::Collector::new();
        let store_ref = Arc::downgrade(&self.0);
        let _ = COLLECTOR.before_scrape(move || Some(store_ref.upgrade()?.scrape_metrics()));

        let res = scope::run!(ctx, |ctx, s| async {
            let persisted = self.0.persistent.persisted();
            let mut queue_next = persisted.borrow().0.next();
            // Task truncating cache whenever a block gets persisted.
            s.spawn::<()>(async {
                let mut persisted = persisted;
                loop {
                    let persisted = sync::changed(ctx, &mut persisted).await?.clone();
                    self.0.inner.send_modify(|inner| {
                        inner.persisted = persisted;
                        inner.truncate_cache();
                    });
                }
            });
            // Task queueing batches to be persisted.
            let inner = &mut self.0.inner.subscribe();
            loop {
                let batch = sync::wait_for(ctx, inner, |inner| inner.queued.0.contains(queue_next))
                    .await?
                    .batch(queue_next)
                    .unwrap();
                queue_next = queue_next.next();

                // TODO: monitor errors as well.
                let t = metrics::PERSISTENT_BLOCK_STORE
                    .queue_next_block_latency
                    .start();
                self.0.persistent.queue_batch(ctx, batch).await?;
                t.observe();
            }
        })
        .await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}

impl BlockStore {
    /// Constructs a BlockStore.
    /// BlockStore takes ownership of the passed PersistentStore,
    /// i.e. caller should modify the underlying persistent storage
    /// ONLY through the constructed BlockStore.
    pub async fn new(
        ctx: &ctx::Ctx,
        persistent: Box<dyn PersistentStore>,
    ) -> ctx::Result<(Arc<Self>, StoreRunner)> {
        let t = metrics::PERSISTENT_BLOCK_STORE.genesis_latency.start();
        let genesis = persistent.genesis(ctx).await.wrap("persistent.genesis()")?;
        t.observe();
        let persisted = persistent.persisted().borrow().clone();
        persisted.0.verify().context("state.verify()")?;
        let this = Arc::new(Self {
            inner: sync::watch::channel(Inner {
                queued: persisted.clone(),
                persisted,
                cache: VecDeque::new(),
            })
            .0,
            genesis,
            persistent,
        });
        Ok((this.clone(), StoreRunner(this)))
    }

    /// Genesis specification for this block store.
    pub fn genesis(&self) -> &validator::Genesis {
        &self.genesis
    }

    /// Available blocks (in memory & persisted).
    pub fn queued(&self) -> (BatchStoreState, BlockStoreState) {
        self.inner.borrow().queued.clone()
    }

    /// Fetches a block (from queue or persistent storage).
    pub async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<Option<validator::FinalBlock>> {
        {
            let inner = self.inner.borrow();
            if !inner.queued.1.contains(number) {
                return Ok(None);
            }
        }
        let t = metrics::PERSISTENT_BLOCK_STORE.block_latency.start();
        let block = self
            .persistent
            .block(ctx, number)
            .await
            .wrap("persistent.block()")?;
        t.observe();
        Ok(Some(block))
    }

    /// Fetches a batch (from queue or persistent storage).
    pub async fn batch(
        &self,
        _ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::SyncBatch>> {
        let inner = self.inner.borrow();
        Ok(inner.batch(number))
    }

    /// Append block to a queue to be persisted eventually.
    /// Since persisting a block may take a significant amount of time,
    /// BlockStore contains a queue of blocks waiting to be persisted.
    /// `queue_block()` adds a block to the queue as soon as all intermediate
    /// blocks are queued_state as well. Queue is unbounded, so it is caller's
    /// responsibility to manage the queue size.
    pub async fn queue_block(
        &self,
        ctx: &ctx::Ctx,
        block: validator::FinalBlock,
    ) -> ctx::Result<()> {
        block.verify(&self.genesis).context("block.verify()")?;
        self.persistent.queue_next_block(ctx, block).await?;
        Ok(())
    }

    /// Append batch to a queue to be persisted eventually.
    /// Since persisting a batch may take a significant amount of time,
    /// BlockStore contains a queue of batches waiting to be persisted.
    /// `queue_batch()` adds a batch to the queue as soon as all intermediate
    /// batches are queued_state as well. Queue is unbounded, so it is caller's
    /// responsibility to manage the queue size.
    pub async fn queue_batch(
        &self,
        _ctx: &ctx::Ctx,
        batch: attester::SyncBatch,
    ) -> ctx::Result<()> {
        self.inner.send_modify(|inner| {
            if inner.try_push(batch.clone()) {
                inner.queued.0.last = Some(batch);
            }
        });
        Ok(())
    }

    /// Waits until the given block is queued (in memory, or persisted).
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    pub async fn wait_until_block_queued(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<BlockStoreState> {
        Ok(sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            number < inner.queued.1.next()
        })
        .await?
        .queued
        .1
        .clone())
    }

    /// Waits until the given block is queued (in memory, or persisted).
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    pub async fn wait_until_batch_queued(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::OrCanceled<BatchStoreState> {
        Ok(sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            number < inner.queued.0.next()
        })
        .await?
        .queued
        .0
        .clone())
    }

    /// Waits until the given block is stored persistently.
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    pub async fn wait_until_block_persisted(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<(BatchStoreState, BlockStoreState)> {
        Ok(
            sync::wait_for(ctx, &mut self.persistent.persisted(), |persisted| {
                number < persisted.1.next()
            })
            .await?
            .clone(),
        )
    }

    /// Waits until the given batch is stored persistently.
    /// Note that it doesn't mean that the batch is actually available, as old batches might get pruned.
    pub async fn wait_until_batch_persisted(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::OrCanceled<(BatchStoreState, BlockStoreState)> {
        Ok(
            sync::wait_for(ctx, &mut self.persistent.persisted(), |persisted| {
                number < persisted.0.next()
            })
            .await?
            .clone(),
        )
    }

    fn scrape_metrics(&self) -> metrics::BlockStore {
        let m = metrics::BlockStore::default();
        let inner = self.inner.borrow();
        m.next_queued_block.set(inner.queued.0.next().0);
        m.next_persisted_block.set(inner.persisted.0.next().0);
        m
    }
}
