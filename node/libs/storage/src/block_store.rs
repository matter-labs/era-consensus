//! Defines storage layer for finalized blocks.
use std::fmt;
use zksync_concurrency::{ctx,sync};
use zksync_consensus_roles::validator;
use std::collections::BTreeMap;

#[derive(Debug,Clone)]
pub struct BlockStoreState {
    pub first: validator::CommitQC,
    pub last: validator::CommitQC,
}

impl BlockStoreState {
    pub fn contains(&self, number: validator::BlockNumber) -> bool {
        self.first.header().number <= number && number <= self.last.header().number 
    }

    pub fn next(&self) -> validator::BlockNumber {
        self.last.header().number.next()
    }
}

/// Storage of a continuous range of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait PersistentBlockStore: fmt::Debug + Send + Sync {
    /// Range of blocks avaliable in storage.
    /// Consensus code calls this method only once and then tracks the
    /// range of avaliable blocks internally.
    async fn state(&self, ctx: &ctx::Ctx) -> ctx::Result<BlockStoreState>;

    /// Gets a block by its number. Should return an error if block is missing.
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
    inmem: sync::watch::Sender<BlockStoreState>,
    persisted: BlockStoreState,
    // TODO: this data structure can be optimized to a VecDeque (or even just a cyclical buffer).
    cache: BTreeMap<validator::BlockNumber,validator::FinalBlock>,
    cache_capacity: usize,
}

#[derive(Debug)]
pub struct BlockStore {
    inner: sync::watch::Sender<Inner>,
    persistent: Box<dyn PersistentBlockStore>,
}

impl BlockStore {
    pub async fn new(ctx: &ctx::Ctx, persistent: Box<dyn PersistentBlockStore>, cache_capacity: usize) -> ctx::Result<Self> {
        if cache_capacity < 1 {
            return Err(anyhow::anyhow!("cache_capacity has to be >=1").into());
        }
        let state = persistent.state(ctx).await?;
        if state.first > state.last {
            return Err(anyhow::anyhow!("at least 1 block has to be available in storage").into());
        }
        Ok(Self {
            persistent,
            inner: sync::watch::channel(Inner{
                inmem: sync::watch::channel(state.clone()).0,
                persisted: state,
                cache: BTreeMap::new(),
                cache_capacity,
            }).0,
        })
    }

    pub async fn block(&self, ctx: &ctx::Ctx, number: validator::BlockNumber) -> ctx::Result<Option<validator::FinalBlock>> {
        {
            let inner = self.inner.borrow();
            if !inner.inmem.borrow().contains(number) {
                return Ok(None);
            }
            if let Some(block) = inner.cache.get(&number) {
                return Ok(Some(block.clone()));
            }
        }
        Ok(Some(self.persistent.block(ctx,number).await?))
    }

    pub async fn last_block(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::FinalBlock> {
        let last = self.inner.borrow().inmem.borrow().last.header().number;
        Ok(self.block(ctx,last).await?.unwrap())
    }

    pub async fn queue_block(&self, ctx: &ctx::Ctx, block: validator::FinalBlock) -> ctx::OrCanceled<()> {
        let number = block.header().number;
        sync::wait_for(ctx, &mut self.subscribe(), |inmem| inmem.next() >= number).await?;
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| inner.cache.len() < inner.cache_capacity).await?;
        self.inner.send_if_modified(|inner| {
            if !inner.inmem.send_if_modified(|inmem| {
                // It may happen that the same block is queued by 2 calls.
                if inmem.next() != number {
                    return false;
                }
                inmem.last = block.justification.clone();
                true
            }) {
                return false;
            }
            inner.cache.insert(number,block);
            true
        });
        Ok(())
    }

    pub async fn store_block(&self, ctx: &ctx::Ctx, block: validator::FinalBlock) -> ctx::OrCanceled<()> {
        let number = block.header().number;
        self.queue_block(ctx,block).await?;
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| inner.persisted.contains(number)).await?;
        Ok(())
    }

    pub fn subscribe(&self) -> sync::watch::Receiver<BlockStoreState> {
        self.inner.borrow().inmem.subscribe()
    }

    pub async fn run_background_tasks(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let res = async { 
            let inner = &mut self.inner.subscribe();
            loop {
                let block = sync::wait_for(ctx, inner, |inner| !inner.cache.is_empty()).await?
                    .cache.first_key_value().unwrap().1.clone();
                self.persistent.store_next_block(ctx,&block).await?;
                self.inner.send_modify(|inner| {
                    debug_assert!(inner.persisted.next()==block.header().number);
                    inner.persisted.last = block.justification.clone();
                    inner.cache.remove(&block.header().number);
                });
            }
        }.await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}
