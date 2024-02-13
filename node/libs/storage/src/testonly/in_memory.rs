//! In-memory storage implementation.
use crate::{BlockStoreState, PersistentBlockStore, ReplicaState};
use anyhow::Context as _;
use std::{collections::VecDeque, sync::Mutex, sync::Arc};
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;


/// In-memory block store.
#[derive(Clone, Debug, Default)]
pub struct BlockStore(Arc<Mutex<VecDeque<validator::FinalBlock>>>);

/// In-memory replica store.
#[derive(Clone, Debug, Default)]
pub struct ReplicaStore(Arc<Mutex<ReplicaState>>);

impl BlockStore {
    /// Creates a new store containing only the specified `genesis_block`.
    pub fn new(genesis: validator::FinalBlock) -> Self {
        Self(Arc::new(Mutex::new([genesis].into())))
    }

    /// Reverts blocks with `number >= next`.
    pub fn revert(&self, next: validator::BlockNumber) {
        let mut blocks = self.0.lock().unwrap();
        if let Some(first) = blocks.front().map(|b|b.header().number.0) {
            blocks.truncate(next.0.saturating_sub(first) as usize);
        }
    }
}

#[async_trait::async_trait]
impl PersistentBlockStore for BlockStore {
    async fn state(&self, _ctx: &ctx::Ctx) -> ctx::Result<BlockStoreState> {
        let blocks = self.0.lock().unwrap();
        if blocks.is_empty() {
            return Err(anyhow::anyhow!("store is empty").into());
        }
        Ok(BlockStoreState {
            first: blocks.front().unwrap().justification.clone(),
            last: blocks.back().unwrap().justification.clone(),
        })
    }

    async fn block(
        &self,
        _ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::FinalBlock> {
        let blocks = self.0.lock().unwrap();
        let front = blocks.front().context("not found")?;
        let idx = number
            .0
            .checked_sub(front.header().number.0)
            .context("not found")?;
        Ok(blocks.get(idx as usize).context("not found")?.clone())
    }

    async fn store_next_block(
        &self,
        _ctx: &ctx::Ctx,
        block: &validator::FinalBlock,
    ) -> ctx::Result<()> {
        let mut blocks = self.0.lock().unwrap();
        let got = block.header().number;
        if let Some(last) = blocks.back() {
            let want = last.header().number.next();
            if got != want {
                return Err(anyhow::anyhow!("got block {got:?}, while expected {want:?}").into());
            }
        }
        blocks.push_back(block.clone());
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
