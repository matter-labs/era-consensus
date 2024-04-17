//! Defines storage layer for finalized blocks.
use anyhow::Context as _;
use std::{collections::VecDeque, fmt, sync::Arc};
use zksync_concurrency::{ctx, error::Wrap as _, scope, sync};
use zksync_consensus_roles::validator;

mod metrics;

/// State of the `BlockStore`: continuous range of blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockStoreState {
    /// Stored block with the lowest number.
    /// If last is `None`, this is the first block that should be fetched.
    pub first: validator::BlockNumber,
    /// Stored block with the highest number.
    /// None iff store is empty.
    pub last: Option<validator::CommitQC>,
}

impl BlockStoreState {
    /// Checks whether block with the given number is stored in the `BlockStore`.
    pub fn contains(&self, number: validator::BlockNumber) -> bool {
        let Some(last) = &self.last else { return false };
        self.first <= number && number <= last.header().number
    }

    /// Number of the next block that can be stored in the `BlockStore`.
    /// (i.e. `last` + 1).
    pub fn next(&self) -> validator::BlockNumber {
        match &self.last {
            Some(qc) => qc.header().number.next(),
            None => self.first,
        }
    }

    /// Verifies `BlockStoreState'.
    pub fn verify(&self, genesis: &validator::Genesis) -> anyhow::Result<()> {
        anyhow::ensure!(
            genesis.fork.first_block <= self.first,
            "first block ({}) doesn't belong to the fork (which starts at block {})",
            self.first,
            genesis.fork.first_block
        );
        if let Some(last) = &self.last {
            anyhow::ensure!(
                self.first <= last.header().number,
                "first block {} has bigger number than the last block {}",
                self.first,
                last.header().number
            );
            last.verify(genesis).context("last.verify()")?;
        }
        Ok(())
    }
}

/// Storage of a continuous range of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait PersistentBlockStore: 'static + fmt::Debug + Send + Sync {
    /// Genesis matching the block store content.
    /// Consensus code calls this method only once.
    async fn genesis(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis>;

    /// Range of blocks persisted in storage.
    fn persisted(&self) -> sync::watch::Receiver<BlockStoreState>;

    /// Gets a block by its number.
    /// All the blocks from `state()` range are expected to be available.
    /// Blocks that have been queued but haven't been persisted yet don't have to be available.
    /// Returns error if block is missing.
    async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::FinalBlock>;

    /// Queue the block to be persisted in storage.
    /// `queue_next_block()` may return BEFORE the block is actually persisted,
    /// but if the call succeeded the block is expected to be persisted eventually.
    /// Implementations are only required to accept a block directly after the previous queued
    /// block, starting with `persisted().borrow().next()`.
    async fn queue_next_block(
        &self,
        ctx: &ctx::Ctx,
        block: validator::FinalBlock,
    ) -> ctx::Result<()>;
}

#[derive(Debug)]
struct Inner {
    queued: BlockStoreState,
    persisted: BlockStoreState,
    cache: VecDeque<validator::FinalBlock>,
}

impl Inner {
    /// Minimal number of most recent blocks to keep in memory.
    /// It allows to serve the recent blocks to peers fast, even
    /// if persistent storage reads are slow (like in RocksDB).
    /// `BlockStore` may keep in memory more blocks in case
    /// blocks are queued faster than they are persisted.
    const CACHE_CAPACITY: usize = 100;

    /// Tries to push the next block to cache.
    /// Noop if provided block is not the expected one.
    /// Returns true iff cache has been modified.
    fn try_push(&mut self, block: validator::FinalBlock) -> bool {
        if self.queued.next() != block.number() {
            return false;
        }
        self.queued.last = Some(block.justification.clone());
        self.cache.push_back(block);
        self.truncate_cache();
        true
    }

    fn truncate_cache(&mut self) {
        while self.cache.len() > Self::CACHE_CAPACITY
            && self.persisted.contains(self.cache[0].number())
        {
            self.cache.pop_front();
        }
    }

    fn block(&self, n: validator::BlockNumber) -> Option<validator::FinalBlock> {
        // Subtraction is safe, because blocks in cache are
        // stored in increasing order of block number.
        let first = self.cache.front()?;
        self.cache.get((n.0 - first.number().0) as usize).cloned()
    }
}

/// A wrapper around a PersistentBlockStore which adds caching blocks in-memory
/// and other useful utilities.
#[derive(Debug)]
pub struct BlockStore {
    inner: sync::watch::Sender<Inner>,
    persistent: Box<dyn PersistentBlockStore>,
    genesis: validator::Genesis,
}

/// Runner of the BlockStore background tasks.
#[must_use]
pub struct BlockStoreRunner(Arc<BlockStore>);

impl BlockStoreRunner {
    /// Runs the background tasks of the BlockStore.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        #[vise::register]
        static COLLECTOR: vise::Collector<Option<metrics::BlockStore>> = vise::Collector::new();
        let store_ref = Arc::downgrade(&self.0);
        let _ = COLLECTOR.before_scrape(move || Some(store_ref.upgrade()?.scrape_metrics()));

        let res = scope::run!(ctx, |ctx, s| async {
            let persisted = self.0.persistent.persisted();
            let mut queue_next = persisted.borrow().next();
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
            // Task queueing blocks to be persisted.
            let inner = &mut self.0.inner.subscribe();
            loop {
                let block = sync::wait_for(ctx, inner, |inner| inner.queued.contains(queue_next))
                    .await?
                    .block(queue_next)
                    .unwrap();
                queue_next = queue_next.next();

                // TODO: monitor errors as well.
                let t = metrics::PERSISTENT_BLOCK_STORE
                    .queue_next_block_latency
                    .start();
                self.0.persistent.queue_next_block(ctx, block).await?;
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
    /// BlockStore takes ownership of the passed PersistentBlockStore,
    /// i.e. caller should modify the underlying persistent storage
    /// ONLY through the constructed BlockStore.
    pub async fn new(
        ctx: &ctx::Ctx,
        persistent: Box<dyn PersistentBlockStore>,
    ) -> ctx::Result<(Arc<Self>, BlockStoreRunner)> {
        let t = metrics::PERSISTENT_BLOCK_STORE.genesis_latency.start();
        let genesis = persistent.genesis(ctx).await.wrap("persistent.genesis()")?;
        t.observe();
        let persisted = persistent.persisted().borrow().clone();
        persisted.verify(&genesis).context("state.verify()")?;
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
        Ok((this.clone(), BlockStoreRunner(this)))
    }

    /// Genesis specification for this block store.
    pub fn genesis(&self) -> &validator::Genesis {
        &self.genesis
    }

    /// Available blocks (in memory & persisted).
    pub fn queued(&self) -> BlockStoreState {
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
            if !inner.queued.contains(number) {
                return Ok(None);
            }
            if let Some(block) = inner.block(number) {
                return Ok(Some(block));
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
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            inner.queued.next() >= block.number()
        })
        .await?;
        self.inner.send_if_modified(|inner| inner.try_push(block));
        Ok(())
    }

    /// Waits until the given block is queued (in memory, or persisted).
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    pub async fn wait_until_queued(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<BlockStoreState> {
        Ok(sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            number < inner.queued.next()
        })
        .await?
        .queued
        .clone())
    }

    /// Waits until the given block is stored persistently.
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    pub async fn wait_until_persisted(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<BlockStoreState> {
        Ok(
            sync::wait_for(ctx, &mut self.persistent.persisted(), |persisted| {
                number < persisted.next()
            })
            .await?
            .clone(),
        )
    }

    fn scrape_metrics(&self) -> metrics::BlockStore {
        let m = metrics::BlockStore::default();
        let inner = self.inner.borrow();
        m.next_queued_block.set(inner.queued.next().0);
        m.next_persisted_block.set(inner.persisted.next().0);
        m
    }
}
