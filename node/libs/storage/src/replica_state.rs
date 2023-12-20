//! `FallbackReplicaStateStore` type.

use crate::{PersistentBlockStore,
    types::ReplicaState,
};
use zksync_concurrency::{ctx,sync};
use zksync_consensus_roles::validator;
use std::collections::BTreeMap;

impl From<validator::CommitQC> for ReplicaState {
    fn from(certificate: validator::CommitQC) -> Self {
        Self {
            view: certificate.message.view,
            phase: validator::Phase::Prepare,
            high_vote: certificate.message,
            high_qc: certificate,
            proposals: vec![],
        }
    }
}

struct BlockStoreInner {
    last_inmem: validator::BlockNumber,
    last_persisted: validator::BlockNumber,
    // TODO: this data structure can be optimized to a VecDeque (or even just a cyclical buffer).
    cache: BTreeMap<validator::BlockNumber,validator::FinalBlock>,
    cache_capacity: usize,
}

pub struct BlockStore {
    inner: sync::watch::Sender<BlockStoreInner>,
    persistent: Box<dyn PersistentBlockStore>,
    first: validator::BlockNumber,
}

impl BlockStore {
    pub async fn new(ctx: &ctx::Ctx, cache_capacity: usize, persistent: Box<dyn PersistentBlockStore>) -> ctx::Result<Self> {
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
            inner: sync::watch::channel(BlockStoreInner{
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

/*
/// Storage combining [`ReplicaStateStore`] and [`WriteBlockStore`].
#[derive(Debug, Clone)]
pub struct ValidatorStore {
    state: Box<dyn ValidatorStore>,
    blocks: Arc<BlockStore>,
}

impl ValidatorStore {
    /// Creates a store from a type implementing both replica state and block storage.
    pub fn from_store<S>(store: Arc<S>) -> Self
    where
        S: ValidatorStore + PersistentBlockStore + 'static,
    {
        Self {
            state: store.clone(),
            blocks: store,
        }
    }

    /// Creates a new replica state store with a fallback.
    pub fn new(state: Arc<dyn ReplicaStateStore>, blocks: Arc<dyn WriteBlockStore>) -> Self {
        Self { state, blocks }
    }

    /// Gets the replica state. If it's not present, falls back to recover it from the fallback block store.
    pub async fn replica_state(&self, ctx: &ctx::Ctx) -> ctx::Result<ReplicaState> {
        let replica_state = self.state.replica_state(ctx).await?;
        if let Some(replica_state) = replica_state {
            Ok(replica_state)
        } else {
            let head_block = self.blocks.head_block(ctx).await?;
            Ok(ReplicaState::from(head_block.justification))
        }
    }
}*/
