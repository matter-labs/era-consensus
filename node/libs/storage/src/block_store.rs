//! Defines storage layer for finalized blocks.
use anyhow::Context as _;
use std::{collections::VecDeque, fmt, sync::Arc};
use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::validator;

/// State of the `BlockStore`: continuous range of blocks.
#[derive(Debug, Clone)]
pub struct BlockStoreState {
    /// Stored block with the lowest number.
    pub first: validator::CommitQC,
    /// Stored block with the highest number.
    pub last: validator::CommitQC,
}

impl BlockStoreState {
    /// Checks whether block with the given number is stored in the `BlockStore`.
    pub fn contains(&self, number: validator::BlockNumber) -> bool {
        self.first.header().number <= number && number <= self.last.header().number
    }

    /// Number of the next block that can be stored in the `BlockStore`.
    /// (i.e. `last` + 1).
    pub fn next(&self) -> validator::BlockNumber {
        self.last.header().number.next()
    }
}

/// Storage of a continuous range of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait PersistentBlockStore: fmt::Debug + Send + Sync {
    /// Range of blocks avaliable in storage. None iff storage is empty.
    /// Consensus code calls this method only once and then tracks the
    /// range of avaliable blocks internally.
    async fn state(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<BlockStoreState>>;

    /// Gets a block by its number. Returns None if block is missing.
    async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<Option<validator::FinalBlock>>;

    /// Persistently store a block.
    /// Implementations are only required to accept a block directly after the current last block,
    /// so that the stored blocks always constitute a continuous range.
    /// Implementation should return only after the block is stored PERSISTENTLY -
    /// consensus liveness property depends on this behavior.
    async fn store_next_block(
        &self,
        ctx: &ctx::Ctx,
        block: &validator::FinalBlock,
    ) -> ctx::Result<()>;
}

#[derive(Debug)]
struct Inner {
    queued: sync::watch::Sender<BlockStoreState>,
    persisted: BlockStoreState,
    queue: VecDeque<validator::FinalBlock>,
}

/// A wrapper around a PersistentBlockStore which adds caching blocks in-memory
/// and other useful utilities.
#[derive(Debug)]
pub struct BlockStore {
    inner: sync::watch::Sender<Inner>,
    persistent: Box<dyn PersistentBlockStore>,
}

/// Runner of the BlockStore background tasks.
#[must_use]
pub struct BlockStoreRunner(Arc<BlockStore>);

impl BlockStoreRunner {
    /// Runs the background tasks of the BlockStore.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let res = async {
            let inner = &mut self.0.inner.subscribe();
            loop {
                let block = sync::wait_for(ctx, inner, |inner| !inner.queue.is_empty())
                    .await?
                    .queue[0]
                    .clone();
                self.0.persistent.store_next_block(ctx, &block).await?;
                self.0.inner.send_modify(|inner| {
                    debug_assert_eq!(inner.persisted.next(), block.header().number);
                    inner.persisted.last = block.justification.clone();
                    inner.queue.pop_front();
                });
            }
        }
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
        let state = persistent
            .state(ctx)
            .await?
            .context("storage empty, expected at least 1 block")?;
        if state.first.header().number > state.last.header().number {
            return Err(anyhow::anyhow!("invalid state").into());
        }
        let this = Arc::new(Self {
            persistent,
            inner: sync::watch::channel(Inner {
                queued: sync::watch::channel(state.clone()).0,
                persisted: state,
                queue: VecDeque::new(),
            })
            .0,
        });
        Ok((this.clone(), BlockStoreRunner(this)))
    }

    /// Fetches a block (from queue or persistent storage).
    pub async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<Option<validator::FinalBlock>> {
        {
            let inner = self.inner.borrow();
            if !inner.queued.borrow().contains(number) {
                return Ok(None);
            }
            if !inner.persisted.contains(number) {
                let idx = number.0 - inner.persisted.next().0;
                return Ok(Some(inner.queue[idx as usize].clone()));
            }
        }
        Ok(Some(
            self.persistent
                .block(ctx, number)
                .await?
                .context("block disappeared from storage")?,
        ))
    }

    /// Insert block to a queue to be persisted eventually.
    /// Since persisting a block may take a significant amount of time,
    /// BlockStore a contains a queue of blocks waiting to be stored.
    /// store_block() adds a block to the queue as soon as all intermediate
    /// blocks are queued as well. Queue is unbounded, so it is caller's
    /// responsibility to manage the queue size.
    pub async fn queue_block(
        &self,
        ctx: &ctx::Ctx,
        block: validator::FinalBlock,
    ) -> ctx::OrCanceled<()> {
        let number = block.header().number;
        sync::wait_for(ctx, &mut self.subscribe(), |queued| queued.next() >= number).await?;
        self.inner.send_if_modified(|inner| {
            let modified = inner.queued.send_if_modified(|queued| {
                // It may happen that the same block is queued by 2 calls.
                if queued.next() != number {
                    return false;
                }
                queued.last = block.justification.clone();
                true
            });
            if !modified {
                return false;
            }
            inner.queue.push_back(block);
            true
        });
        Ok(())
    }

    /// Waits until the given block is queued to be stored.
    pub async fn wait_until_queued(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<()> {
        sync::wait_for(ctx, &mut self.subscribe(), |queued| queued.contains(number)).await?;
        Ok(())
    }

    /// Waits until the given block is stored persistently.
    pub async fn wait_until_stored(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<()> {
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            inner.persisted.contains(number)
        })
        .await?;
        Ok(())
    }

    /// Subscribes to the `BlockStoreState` changes.
    /// Note that this state includes both queue AND stored blocks.
    pub fn subscribe(&self) -> sync::watch::Receiver<BlockStoreState> {
        self.inner.borrow().queued.subscribe()
    }
}
