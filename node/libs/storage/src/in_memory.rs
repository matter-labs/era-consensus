//! In-memory storage implementation.
use crate::{
    traits,
    types::{ReplicaState},
};
use anyhow::Context as _;
use std::{ops, sync::Mutex};
use zksync_concurrency::{ctx};
use zksync_consensus_roles::validator;

/// In-memory store.
#[derive(Debug)]
pub struct InMemoryStorage {
    blocks: Mutex<Vec<validator::FinalBlock>>,
    replica_state: Mutex<Option<ReplicaState>>,
}

impl InMemoryStorage {
    /// Creates a new store containing only the specified `genesis_block`.
    pub fn new(genesis_block: validator::FinalBlock) -> Self {
        Self {
            blocks: Mutex::new(vec![genesis_block]),
            replica_state: Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl traits::BlockStore for InMemoryStorage {
    async fn available_blocks(&self, _ctx :&ctx::Ctx) -> ctx::Result<ops::Range<validator::BlockNumber>> {
        let blocks = self.blocks.lock().unwrap();
        Ok(ops::Range {
            start: blocks.first().unwrap().header().number, 
            end: blocks.last().unwrap().header().number.next(),
        })
    }

    async fn block(&self, _ctx: &ctx::Ctx, number: validator::BlockNumber) -> ctx::Result<validator::FinalBlock> {
        let blocks = self.blocks.lock().unwrap();
        let first = blocks.first().unwrap().header().number;
        if number < first {
            return Err(anyhow::anyhow!("not found").into());
        }
        Ok(blocks.get((number.0-first.0) as usize).context("not found")?.clone())
    }

    async fn store_next_block(&self, _ctx: &ctx::Ctx, block: &validator::FinalBlock) -> ctx::Result<()> {
        let mut blocks = self.blocks.lock().unwrap();
        let got = block.header().number;
        let want = blocks.last().unwrap().header().number.next();
        if got != want {
            return Err(anyhow::anyhow!("got block {got:?}, while expected {want:?}").into());
        }
        blocks.push(block.clone());
        Ok(())
    }    
}

#[async_trait::async_trait]
impl traits::ReplicaStore for InMemoryStorage {
    async fn state(&self, _ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>> {
        Ok(self.replica_state.lock().unwrap().clone())
    }

    async fn set_state(&self, _ctx: &ctx::Ctx, replica_state: &ReplicaState) -> ctx::Result<()> {
        *self.replica_state.lock().unwrap() = Some(replica_state.clone());
        Ok(())
    }
}
