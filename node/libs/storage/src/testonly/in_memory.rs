//! In-memory storage implementation.
use crate::{
    batch_store::BatchStoreState, BlockStoreState, PersistentBatchStore, PersistentBlockStore,
    ReplicaState,
};
use anyhow::Context as _;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::{attester, validator};

#[derive(Debug)]
struct BlockStoreInner {
    genesis: validator::Genesis,
    persisted: sync::watch::Sender<BlockStoreState>,
    blocks: Mutex<VecDeque<validator::FinalBlock>>,
}

#[derive(Debug)]
struct BatchStoreInner {
    persisted: sync::watch::Sender<BatchStoreState>,
    batches: Mutex<VecDeque<attester::FinalBatch>>,
}

/// In-memory block store.
#[derive(Clone, Debug)]
pub struct BlockStore(Arc<BlockStoreInner>);

/// In-memory replica store.
#[derive(Clone, Debug, Default)]
pub struct ReplicaStore(Arc<Mutex<ReplicaState>>);

/// In-memory replica store.
#[derive(Clone, Debug)]
pub struct BatchStore(Arc<BatchStoreInner>);

impl BlockStore {
    /// New In-memory `BlockStore`.
    pub fn new(genesis: validator::Genesis, first: validator::BlockNumber) -> Self {
        assert!(genesis.first_block <= first);
        Self(Arc::new(BlockStoreInner {
            genesis,
            persisted: sync::watch::channel(BlockStoreState { first, last: None }).0,
            blocks: Mutex::default(),
        }))
    }
}

impl BatchStore {
    /// New In-memory `BatchStore`.
    pub fn new(first: attester::BatchNumber) -> Self {
        Self(Arc::new(BatchStoreInner {
            persisted: sync::watch::channel(BatchStoreState { first, last: None }).0,
            batches: Mutex::default(),
        }))
    }
}

#[async_trait::async_trait]
impl PersistentBlockStore for BlockStore {
    async fn genesis(&self, _ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis> {
        Ok(self.0.genesis.clone())
    }

    fn persisted(&self) -> sync::watch::Receiver<BlockStoreState> {
        self.0.persisted.subscribe()
    }

    async fn block(
        &self,
        _ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::FinalBlock> {
        let blocks = self.0.blocks.lock().unwrap();
        let front = blocks.front().context("not found")?;
        let idx = number
            .0
            .checked_sub(front.header().number.0)
            .context("not found")?;
        Ok(blocks.get(idx as usize).context("not found")?.clone())
    }

    async fn queue_next_block(
        &self,
        _ctx: &ctx::Ctx,
        block: validator::FinalBlock,
    ) -> ctx::Result<()> {
        let mut blocks = self.0.blocks.lock().unwrap();
        let want = self.0.persisted.borrow().next();
        if block.number() != want {
            return Err(anyhow::anyhow!("got block {:?}, want {want:?}", block.number()).into());
        }
        self.0
            .persisted
            .send_modify(|p| p.last = Some(block.justification.clone()));
        blocks.push_back(block);
        Ok(())
    }
}

#[async_trait::async_trait]
impl PersistentBatchStore for BatchStore {
    fn persisted(&self) -> sync::watch::Receiver<BatchStoreState> {
        self.0.persisted.subscribe()
    }

    fn last_batch(&self) -> attester::BatchNumber {
        self.0
            .persisted
            .borrow()
            .last
            .clone()
            .map(|qc| qc.message.proposal.number)
            .unwrap()
    }

    fn last_batch_qc(&self) -> attester::BatchQC {
        self.0.persisted.borrow().last.clone().unwrap()
    }

    fn get_batch(&self, number: attester::BatchNumber) -> Option<attester::FinalBatch> {
        let batches = self.0.batches.lock().unwrap();
        let front = batches.front()?;
        let idx = number.0.checked_sub(front.number().0)?;
        batches.get(idx as usize).cloned()
    }

    fn get_batch_qc(&self, number: attester::BatchNumber) -> Option<attester::BatchQC> {
        let batch = self.get_batch(number)?;
        Some(batch.justification.clone())
    }

    fn store_qc(&self, qc: attester::BatchQC) {
        self.0.persisted.send_modify(|p| p.last = Some(qc.clone()));
    }

    async fn queue_next_batch(
        &self,
        _ctx: &ctx::Ctx,
        batch: attester::FinalBatch,
    ) -> ctx::Result<()> {
        let mut batches = self.0.batches.lock().unwrap();
        let want = self.0.persisted.borrow().next();
        if batch.number() != want {
            return Err(anyhow::anyhow!("got batch {:?}, want {want:?}", batch.number()).into());
        }
        self.0
            .persisted
            .send_modify(|p| p.last = Some(batch.justification.clone()));
        batches.push_back(batch);
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::ReplicaStore for ReplicaStore {
    async fn state(&self, _ctx: &ctx::Ctx) -> ctx::Result<ReplicaState> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn set_state(&self, _ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()> {
        *self.0.lock().unwrap() = state.clone();
        Ok(())
    }
}
