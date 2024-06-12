//! In-memory storage implementation.
use anyhow::Context as _;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::{attester, validator};

use crate::{
    store::{BatchStoreState, BlockStoreState, PersistentStore},
    ReplicaState,
};

#[derive(Debug)]
struct StoreInner {
    genesis: validator::Genesis,
    persisted: sync::watch::Sender<(BatchStoreState, BlockStoreState)>,
    batches: Mutex<VecDeque<attester::SyncBatch>>,
    blocks: Mutex<VecDeque<validator::FinalBlock>>,
}

/// In-memory replica store.
#[derive(Clone, Debug, Default)]
pub struct ReplicaStore(Arc<Mutex<ReplicaState>>);

/// In-memory replica store.
#[derive(Clone, Debug)]
pub struct BlockStore(Arc<StoreInner>);

impl BlockStore {
    /// New In-memory `BlockStore`.
    pub fn new(genesis: validator::Genesis, first: validator::BlockNumber) -> Self {
        Self(Arc::new(StoreInner {
            genesis: genesis.clone(),
            persisted: sync::watch::channel((
                BatchStoreState {
                    first: attester::BatchNumber(0),
                    last: None,
                },
                BlockStoreState { first, last: None },
            ))
            .0,
            batches: Mutex::default(),
            blocks: Mutex::default(),
        }))
    }
}

#[async_trait::async_trait]
impl PersistentStore for BlockStore {
    async fn genesis(&self, _ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis> {
        Ok(self.0.genesis.clone())
    }

    async fn batch(&self, number: attester::BatchNumber) -> ctx::Result<attester::SyncBatch> {
        let batches = self.0.batches.lock().unwrap();
        let front = batches.front().context("not found")?;
        let idx = number.0.checked_sub(front.number.0).context("not found")?;
        Ok(batches.get(idx as usize).context("not found")?.clone())
    }

    async fn queue_batch(&self, _ctx: &ctx::Ctx, batch: attester::SyncBatch) -> ctx::Result<()> {
        let mut batches = self.0.batches.lock().unwrap();
        let want = self.0.persisted.borrow().0.next();
        if batch.number != want {
            return Err(anyhow::anyhow!("got batch {:?}, want {want:?}", batch.number).into());
        }
        self.0
            .persisted
            .send_modify(|p| p.0.last = Some(batch.clone()));
        batches.push_back(batch);
        Ok(())
    }

    fn persisted(&self) -> sync::watch::Receiver<(BatchStoreState, BlockStoreState)> {
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
        let want = self.0.persisted.borrow().1.next();
        if block.number() != want {
            return Err(anyhow::anyhow!("got block {:?}, want {want:?}", block.number()).into());
        }
        self.0
            .persisted
            .send_modify(|p| p.1.last = Some(block.justification.clone()));
        blocks.push_back(block);
        if blocks.len() % 5 == 0 {
            self.0.persisted.send_modify(|b| {
                b.0.last = Some(attester::SyncBatch {
                    number: b
                        .0
                        .last
                        .clone()
                        .map(|l| l.number)
                        .unwrap_or(b.0.first)
                        .next(),
                    payloads: blocks.iter().take(5).map(|b| b.payload.clone()).collect(),
                    proof: Vec::default(),
                });
                self.0
                    .batches
                    .lock()
                    .unwrap()
                    .push_back(b.0.last.as_ref().unwrap().clone());
            });
        }
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
