//! In-memory storage implementation.
use crate::{block_store::Last, BlockStoreState, PersistentBlockStore, ReplicaState};
use anyhow::Context as _;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::{validator, validator::testonly::Setup};

#[derive(Debug)]
struct BlockStoreInner {
    genesis: validator::Genesis,
    persisted: sync::watch::Sender<BlockStoreState>,
    blocks: Mutex<VecDeque<validator::Block>>,
    pre_genesis_blocks: HashMap<validator::BlockNumber, validator::PreGenesisBlock>,
}

/// In-memory block store.
#[derive(Clone, Debug)]
pub struct BlockStore(Arc<BlockStoreInner>);

/// In-memory replica store.
#[derive(Clone, Debug, Default)]
pub struct ReplicaStore(Arc<Mutex<ReplicaState>>);

impl BlockStore {
    /// New In-memory `BlockStore`.
    pub fn new(setup: &Setup, first: validator::BlockNumber) -> Self {
        assert!(
            setup
                .blocks
                .first()
                .map(|b| b.number())
                .unwrap_or(setup.genesis.first_block)
                <= first
        );
        Self(Arc::new(BlockStoreInner {
            genesis: setup.genesis.clone(),
            persisted: sync::watch::channel(BlockStoreState { first, last: None }).0,
            blocks: Mutex::default(),
            pre_genesis_blocks: setup
                .blocks
                .iter()
                .flat_map(|b| match b {
                    validator::Block::PreGenesis(b) => Some((b.number, b.clone())),
                    validator::Block::Final(_) => None,
                })
                .collect(),
        }))
    }

    /// Truncates the storage to blocks `>=first`.
    pub fn truncate(&mut self, first: validator::BlockNumber) {
        let mut blocks = self.0.blocks.lock().unwrap();
        while blocks.front().map_or(false, |b| b.number() < first) {
            blocks.pop_front();
        }
        self.0.persisted.send_if_modified(|s| {
            if s.first >= first {
                return false;
            }
            if s.next() <= first {
                s.last = None;
            }
            s.first = first;
            true
        });
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

    async fn verify_pre_genesis_block(
        &self,
        _ctx: &ctx::Ctx,
        block: &validator::PreGenesisBlock,
    ) -> ctx::Result<()> {
        if self.0.pre_genesis_blocks.get(&block.number) != Some(block) {
            return Err(anyhow::format_err!("invalid pre-genesis block").into());
        }
        Ok(())
    }

    async fn block(
        &self,
        _ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Block> {
        let blocks = self.0.blocks.lock().unwrap();
        let front = blocks.front().context("not found")?;
        let idx = number
            .0
            .checked_sub(front.number().0)
            .context("not found")?;
        Ok(blocks.get(idx as usize).context("not found")?.clone())
    }

    async fn queue_next_block(&self, ctx: &ctx::Ctx, block: validator::Block) -> ctx::Result<()> {
        if let validator::Block::PreGenesis(b) = &block {
            self.verify_pre_genesis_block(ctx, b).await?;
        }
        let mut blocks = self.0.blocks.lock().unwrap();
        let want = self.0.persisted.borrow().next();
        if block.number() < want {
            // It may happen that a block gets fetched which is not needed any more.
            return Ok(());
        }
        if block.number() > want {
            // Blocks should be stored in order though.
            return Err(anyhow::anyhow!("got block {:?}, want {want:?}", block.number()).into());
        }
        self.0
            .persisted
            .send_modify(|p| p.last = Some(Last::from(&block)));
        blocks.push_back(block);
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
