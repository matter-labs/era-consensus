//! Defines storage layer for finalized blocks.
use std::fmt;
use std::ops;
use zksync_concurrency::{ctx,sync};
use zksync_consensus_roles::validator;
use std::collections::BTreeMap;

/// Storage of a continuous range of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait PersistentBlockStore: fmt::Debug + Send + Sync {
    /// Range of blocks avaliable in storage.
    /// Consensus code calls this method only once and then tracks the
    /// range of avaliable blocks internally.
    async fn available_blocks(&self, ctx: &ctx::Ctx) -> ctx::Result<ops::Range<validator::BlockNumber>>;

    /// Gets a block by its number. Should returns an error if block is not available.
    async fn block(&self, ctx: &ctx::Ctx, number: validator::BlockNumber) -> ctx::Result<validator::FinalBlock>;

    /// Persistently store a block.
    /// Implementations are only required to accept a block directly after the current last block,
    /// so that the stored blocks always constitute a continuous range.
    /// Implementation should return only after the block is stored PERSISTENTLY -
    /// consensus liveness property depends on this behavior.
    async fn store_next_block(&self, ctx: &ctx::Ctx, block: &validator::FinalBlock) -> ctx::Result<()>;
}

#[derive(Debug)]
struct Inner {
    last_inmem: validator::BlockNumber,
    last_persisted: validator::BlockNumber,
    // TODO: this data structure can be optimized to a VecDeque (or even just a cyclical buffer).
    cache: BTreeMap<validator::BlockNumber,validator::FinalBlock>,
    cache_capacity: usize,
}

#[derive(Debug)]
pub struct BlockStore {
    inner: sync::watch::Sender<Inner>,
    persistent: Box<dyn PersistentBlockStore>,
    first: validator::BlockNumber,
}

impl BlockStore {
    pub async fn new(ctx: &ctx::Ctx, persistent: Box<dyn PersistentBlockStore>, cache_capacity: usize) -> ctx::Result<Self> {
        if cache_capacity < 1 {
            return Err(anyhow::anyhow!("cache_capacity has to be >=1").into());
        }
        let avail = persistent.available_blocks(ctx).await?;
        if avail.start >= avail.end {
            return Err(anyhow::anyhow!("at least 1 block has to be available in storage").into());
        }
        let last = avail.end.prev();
        Ok(Self {
            persistent,
            first: avail.start,
            inner: sync::watch::channel(Inner{
                last_inmem: last,
                last_persisted: last,
                cache: BTreeMap::new(),
                cache_capacity,
            }).0,
        })
    }

    pub async fn block(&self, ctx: &ctx::Ctx, number: validator::BlockNumber) -> ctx::Result<Option<validator::FinalBlock>> {
        if number < self.first {
            return Ok(None);
        }
        {
            let inner = self.inner.borrow();
            if inner.last_inmem > number {
                return Ok(None);
            }
            if inner.last_persisted < number {
                return Ok(inner.cache.get(&number).cloned());
            }
        }
        Ok(Some(self.persistent.block(ctx,number).await?))
    }

    pub async fn wait_for_block(&self, ctx: &ctx::Ctx, number: validator::BlockNumber) -> ctx::Result<validator::FinalBlock> {
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| inner.last_inmem >= number).await?;
        Ok(self.block(ctx,number).await?.unwrap())
    }

    pub async fn last_block(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::FinalBlock> {
        let last = self.inner.borrow().last_inmem;
        Ok(self.block(ctx,last).await?.unwrap())
    }

    pub async fn queue_block(&self, ctx: &ctx::Ctx, block: validator::FinalBlock) -> ctx::Result<()> {
        let number = block.header().number;
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            inner.last_inmem.next() >= number && (number.0-inner.last_persisted.0) as usize <= inner.cache_capacity
        }).await?;
        self.inner.send_if_modified(|inner| {
            // It may happen that the same block is queued by 2 calls.
            if inner.last_inmem.next() != number {
                return false;
            }
            inner.cache.insert(number,block);
            inner.last_inmem = number;
            true
        });
        Ok(())
    }

    pub async fn store_block(&self, ctx: &ctx::Ctx, block: validator::FinalBlock) -> ctx::Result<()> {
        let number = block.header().number;
        self.queue_block(ctx,block).await?;
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| inner.last_persisted >= number).await?;
        Ok(())
    }

    pub async fn run_background_tasks(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let res = async {
            let inner = &mut self.inner.subscribe();
            loop {
                let block = sync::wait_for(ctx, inner, |inner| !inner.cache.is_empty()).await?.cache.first_key_value()
                    .unwrap().1.clone();
                self.persistent.store_next_block(ctx,&block).await?;
                self.inner.send_modify(|inner| {
                    debug_assert!(inner.last_persisted.next()==block.header().number);
                    inner.last_persisted = block.header().number;
                    inner.cache.remove(&inner.last_persisted);
                });
            }
        }.await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}
