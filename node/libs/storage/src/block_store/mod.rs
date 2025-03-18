//! Defines storage layer for finalized blocks.
use std::{collections::VecDeque, fmt, sync::Arc};

use anyhow::Context as _;
use tracing::Instrument;
use zksync_concurrency::{ctx, error::Wrap as _, scope, sync};
use zksync_consensus_roles::validator;

mod metrics;

/// Last block in the block store (see `BlockStoreState`).
/// Note that the commit qc is required in the case the block
/// has been finalized by the consensus.
#[derive(Debug, Clone, PartialEq)]
pub enum Last {
    /// `<genesis.first_block`.
    PreGenesis(validator::BlockNumber),
    /// `>=genesis.first_block`.
    FinalV1(validator::v1::CommitQC),
    /// `>=genesis.first_block`.
    FinalV2(validator::v2::CommitQC),
}

impl From<&validator::Block> for Last {
    fn from(b: &validator::Block) -> Last {
        use validator::Block as B;
        match b {
            B::PreGenesis(b) => Last::PreGenesis(b.number),
            B::FinalV1(b) => Last::FinalV1(b.justification.clone()),
            B::FinalV2(b) => Last::FinalV2(b.justification.clone()),
        }
    }
}

impl Last {
    /// Converts Last to block number.
    pub fn number(&self) -> validator::BlockNumber {
        match self {
            Last::PreGenesis(n) => *n,
            Last::FinalV1(qc) => qc.header().number,
            Last::FinalV2(qc) => qc.header().number,
        }
    }

    /// Verifies Last.
    pub fn verify(&self, genesis: &validator::Genesis) -> anyhow::Result<()> {
        match self {
            Last::PreGenesis(n) => anyhow::ensure!(n < &genesis.first_block, "missing qc"),
            Last::FinalV1(qc) => qc.verify(genesis)?,
            Last::FinalV2(qc) => qc.verify(genesis)?,
        }
        Ok(())
    }
}

/// State of the `BlockStore`: continuous range of blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockStoreState {
    /// Stored block with the lowest number.
    /// If `last` is `None`, this is the first block that should be fetched.
    pub first: validator::BlockNumber,
    /// Stored block with the highest number.
    /// If it is lower than genesis.first, then it will be set to `Last::Number` and
    /// the first blocks to be fetched will need to include the non-consensus justification
    /// (see `PreGenesisBlock`).
    /// None iff store is empty.
    pub last: Option<Last>,
}

impl BlockStoreState {
    /// Checks whether block with the given number is stored in the `BlockStore`.
    pub fn contains(&self, number: validator::BlockNumber) -> bool {
        let Some(last) = &self.last else { return false };
        self.first <= number && number <= last.number()
    }

    /// Number of the next block that can be stored in the `BlockStore`.
    /// (i.e. `last` + 1).
    pub fn next(&self) -> validator::BlockNumber {
        match &self.last {
            Some(last) => last.number().next(),
            None => self.first,
        }
    }

    /// Verifies `BlockStoreState'.
    pub fn verify(&self, genesis: &validator::Genesis) -> anyhow::Result<()> {
        if let Some(last) = &self.last {
            anyhow::ensure!(
                self.first <= last.number(),
                "first block {} has bigger number than the last block {}",
                self.first,
                last.number(),
            );
            last.verify(genesis).context("last")?;
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

    /// Verifies a pre-genesis block.
    /// It may interpret `block.justification`
    /// and/or consult external source of truth.
    async fn verify_pregenesis_block(
        &self,
        ctx: &ctx::Ctx,
        block: &validator::PreGenesisBlock,
    ) -> ctx::Result<()>;

    /// Gets a block by its number.
    /// All the blocks from `state()` range are expected to be available.
    /// Blocks that have been queued but haven't been persisted yet don't have to be available.
    /// Returns error if block is missing.
    async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Block>;

    /// Queue the block to be persisted in storage.
    /// `queue_next_block()` may return BEFORE the block is actually persisted,
    /// but if the call succeeded the block is expected to be persisted eventually.
    /// Implementations are only required to accept a block directly after the previous queued
    /// block, starting with `persisted().borrow().next()`.
    async fn queue_next_block(&self, ctx: &ctx::Ctx, block: validator::Block) -> ctx::Result<()>;
}

#[derive(Debug)]
struct Inner {
    queued: BlockStoreState,
    persisted: BlockStoreState,
    cache: VecDeque<validator::Block>,
}

/// Minimal number of most recent blocks to keep in memory.
/// It allows to serve the recent blocks to peers fast, even
/// if persistent storage reads are slow (like in RocksDB).
/// `BlockStore` may keep in memory more blocks in case
/// blocks are queued faster than they are persisted.
pub(crate) const CACHE_CAPACITY: usize = 100;

impl Inner {
    /// Tries to push the next block to cache.
    /// Noop if provided block is not the expected one.
    /// Returns true iff cache has been modified.
    fn try_push(&mut self, block: validator::Block) -> bool {
        if self.queued.next() != block.number() {
            return false;
        }
        self.queued.last = Some(Last::from(&block));
        self.cache.push_back(block);
        self.truncate_cache();
        true
    }

    /// Updates `persisted` field.
    #[tracing::instrument(skip_all)]
    fn update_persisted(&mut self, persisted: BlockStoreState) -> anyhow::Result<()> {
        if persisted.next() < self.persisted.next() {
            anyhow::bail!("head block has been removed from storage, this is not supported");
        }
        self.persisted = persisted;
        if self.queued.first < self.persisted.first {
            self.queued.first = self.persisted.first;
        }
        // If persisted blocks overtook the queue (blocks were fetched via some side-channel),
        // it means we need to reset the cache - otherwise we would have a gap.
        if self.queued.next() < self.persisted.next() {
            self.queued = self.persisted.clone();
            self.cache.clear();
        }
        self.truncate_cache();
        Ok(())
    }

    /// If cache size has been exceeded, remove entries which were already persisted.
    fn truncate_cache(&mut self) {
        while self.cache.len() > CACHE_CAPACITY && self.persisted.next() > self.cache[0].number() {
            self.cache.pop_front();
        }
    }

    fn block(&self, n: validator::BlockNumber) -> Option<validator::Block> {
        let first = self.cache.front()?;
        self.cache
            .get(n.0.checked_sub(first.number().0)? as usize)
            .cloned()
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
    ) -> ctx::Result<Option<validator::Block>> {
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

    /// Verifies the block.
    pub async fn verify_block(&self, ctx: &ctx::Ctx, block: &validator::Block) -> ctx::Result<()> {
        use validator::Block as B;
        match &block {
            B::PreGenesis(b) => {
                if b.number >= self.genesis.first_block {
                    return Err(anyhow::format_err!(
                        "external justification is allowed only for pre-genesis blocks"
                    )
                    .into());
                }
                let t = metrics::PERSISTENT_BLOCK_STORE
                    .verify_pregenesis_block_latency
                    .start();
                self.persistent
                    .verify_pregenesis_block(ctx, b)
                    .await
                    .context("verify_pregenesis_block()")?;
                t.observe();
            }
            B::FinalV1(b) => b.verify(&self.genesis).context("block.verify()")?,
            B::FinalV2(b) => b.verify(&self.genesis).context("block.verify()")?,
        }
        Ok(())
    }

    /// Append block to a queue to be persisted eventually.
    /// Since persisting a block may take a significant amount of time,
    /// BlockStore contains a queue of blocks waiting to be persisted.
    /// `queue_block()` adds a block to the queue as soon as all intermediate
    /// blocks are queued_state as well. Queue is unbounded, so it is caller's
    /// responsibility to manage the queue size.
    pub async fn queue_block(&self, ctx: &ctx::Ctx, block: validator::Block) -> ctx::Result<()> {
        self.verify_block(ctx, &block)
            .await
            .with_wrap(|| format!("verify_block({})", block.number()))?;
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            inner.queued.next() >= block.number()
        })
        .await?;
        self.inner.send_if_modified(|inner| inner.try_push(block));
        Ok(())
    }

    /// Waits until the queued blocks range is different than `old`.
    pub async fn wait_for_queued_change(
        &self,
        ctx: &ctx::Ctx,
        old: &BlockStoreState,
    ) -> ctx::OrCanceled<BlockStoreState> {
        sync::wait_for_some(ctx, &mut self.inner.subscribe(), |inner| {
            if &inner.queued == old {
                return None;
            }
            Some(inner.queued.clone())
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
        Ok(sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            number < inner.queued.next()
        })
        .await?
        .queued
        .clone())
    }

    /// Waits until the given block is stored persistently.
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    #[tracing::instrument(skip_all, fields(l2_block = %number))]
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

    fn scrape_metrics(&self) -> metrics::BlockStoreState {
        let m = metrics::BlockStoreState::default();
        let inner = self.inner.borrow();
        m.next_queued_block.set(inner.queued.next().0);
        m.next_persisted_block.set(inner.persisted.next().0);
        m
    }
}

/// Runner of the BlockStore background tasks.
#[must_use]
#[derive(Debug, Clone)]
pub struct BlockStoreRunner(Arc<BlockStore>);

impl BlockStoreRunner {
    /// Runs the background tasks of the BlockStore.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        #[vise::register]
        static COLLECTOR: vise::Collector<Option<metrics::BlockStoreState>> =
            vise::Collector::new();
        let store_ref = Arc::downgrade(&self.0);
        let _ = COLLECTOR.before_scrape(move || Some(store_ref.upgrade()?.scrape_metrics()));

        let res = scope::run!(ctx, |ctx, s| async {
            s.spawn::<()>(async {
                // Task watching the persisted state.
                let mut persisted = self.0.persistent.persisted();
                persisted.mark_changed();
                loop {
                    async {
                        let new = sync::changed(ctx, &mut persisted)
                            .instrument(tracing::info_span!("wait_for_block_store_change"))
                            .await?
                            .clone();
                        sync::try_send_modify(&self.0.inner, |inner| inner.update_persisted(new))?;

                        ctx::Ok(())
                    }
                    .instrument(tracing::info_span!("watch_persistent_state_iteration"))
                    .await?;
                }
            });
            // Task queueing blocks to be persisted.
            let inner = &mut self.0.inner.subscribe();
            let mut queue_next = validator::BlockNumber(0);
            loop {
                async {
                    let block = sync::wait_for_some(ctx, inner, |inner| {
                        inner.block(queue_next.max(inner.persisted.next()))
                    })
                    .instrument(tracing::info_span!("wait_for_next_block"))
                    .await?;
                    queue_next = block.number().next();
                    // TODO: monitor errors as well.
                    let t = metrics::PERSISTENT_BLOCK_STORE
                        .queue_next_block_latency
                        .start();
                    self.0.persistent.queue_next_block(ctx, block).await?;
                    t.observe();

                    ctx::Ok(())
                }
                .instrument(tracing::info_span!("queue_persist_block_iteration"))
                .await?;
            }
        })
        .await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}
