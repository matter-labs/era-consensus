//! In-memory storage implementation.
use crate::{BlockStoreState, PersistentBlockStore, ReplicaState};
use std::collections::BTreeMap;
use std::sync::Mutex;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;

/// In-memory block store.
#[derive(Debug,Default)]
pub struct BlockStore(Mutex<BTreeMap<validator::BlockNumber, validator::FinalBlock>>);

/// In-memory replica store.
#[derive(Debug, Default)]
pub struct ReplicaStore(Mutex<Option<ReplicaState>>);

impl BlockStore {
    /// Creates a new store containing only the specified `genesis_block`.
    pub fn new(genesis: validator::FinalBlock) -> Self {
        Self(Mutex::new([(genesis.header().number, genesis)].into()))
    }
}

#[async_trait::async_trait]
impl PersistentBlockStore for BlockStore {
    async fn state(&self, _ctx: &ctx::Ctx) -> ctx::Result<Option<BlockStoreState>> {
        let blocks = self.0.lock().unwrap();
        if blocks.is_empty() { return Ok(None) } 
        Ok(Some(BlockStoreState {
            first: blocks.first_key_value().unwrap().1.justification.clone(),
            last: blocks.last_key_value().unwrap().1.justification.clone(),
        }))
    }

    async fn block(
        &self,
        _ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<Option<validator::FinalBlock>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&number)
            .cloned())
    }

    async fn store_next_block(
        &self,
        _ctx: &ctx::Ctx,
        block: &validator::FinalBlock,
    ) -> ctx::Result<()> {
        let mut blocks = self.0.lock().unwrap();
        let got = block.header().number;
        if !blocks.is_empty() {
            let want = blocks.last_key_value().unwrap().0.next();
            if got != want {
                return Err(anyhow::anyhow!("got block {got:?}, while expected {want:?}").into());
            }
        }
        blocks.insert(got, block.clone());
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::ReplicaStore for ReplicaStore {
    async fn state(&self, _ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn set_state(&self, _ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()> {
        *self.0.lock().unwrap() = Some(state.clone());
        Ok(())
    }
}
