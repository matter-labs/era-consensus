//! In-memory storage implementation.
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use anyhow::Context as _;
use rand::Rng;
use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::validator::{self, testonly::Setup, ReplicaState};

use crate::{block_store::Last, BlockStoreState, EngineInterface};

/// In-memory engine manager.
#[derive(Clone, Debug)]
pub struct Engine(Arc<EngineInner>);

impl Engine {
    /// New in-memory `EngineManager` with a random payload manager.
    pub fn new_random(setup: &Setup, first: validator::BlockNumber) -> Self {
        Self::new(setup, first, PayloadManager::Random(100))
    }

    /// New in-memory `EngineManager` with a pending payload manager.
    pub fn new_pending(setup: &Setup, first: validator::BlockNumber) -> Self {
        Self::new(setup, first, PayloadManager::Pending)
    }

    /// New in-memory `EngineManager` with a rejecting payload manager.
    pub fn new_reject(setup: &Setup, first: validator::BlockNumber) -> Self {
        Self::new(setup, first, PayloadManager::Reject)
    }

    /// New in-memory `EngineManager`.
    pub fn new(
        setup: &Setup,
        first: validator::BlockNumber,
        payload_manager: PayloadManager,
    ) -> Self {
        assert!(
            setup
                .blocks
                .first()
                .map(|b| b.number())
                .unwrap_or(setup.genesis.first_block)
                <= first
        );
        Self(Arc::new(EngineInner {
            genesis: setup.genesis.clone(),
            persisted: sync::watch::channel(BlockStoreState { first, last: None }).0,
            blocks: Mutex::default(),
            pregenesis_blocks: setup
                .blocks
                .iter()
                .flat_map(|b| match b {
                    validator::Block::PreGenesis(b) => Some((b.number, b.clone())),
                    _ => None,
                })
                .collect(),
            payload_manager,
            state: Arc::new(Mutex::new(ReplicaState::default())),
            capacity: None,
        }))
    }

    /// New in-memory `EngineManager` with a bounded storage.
    /// Old blocks get garbage collected once the storage capacity is full.
    pub fn new_bounded(
        genesis: validator::Genesis,
        first: validator::BlockNumber,
        capacity: usize,
    ) -> Self {
        assert!(genesis.first_block <= first);
        Self(Arc::new(EngineInner {
            genesis,
            persisted: sync::watch::channel(BlockStoreState { first, last: None }).0,
            blocks: Mutex::default(),
            pregenesis_blocks: [].into(),
            payload_manager: PayloadManager::Random(100),
            state: Arc::new(Mutex::new(ReplicaState::default())),
            capacity: Some(capacity),
        }))
    }

    /// Truncates the storage to blocks `>=first`.
    pub fn truncate(&mut self, first: validator::BlockNumber) {
        let mut blocks = self.0.blocks.lock().unwrap();
        while blocks.front().is_some_and(|b| b.number() < first) {
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
impl EngineInterface for Engine {
    async fn genesis(&self, _ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis> {
        Ok(self.0.genesis.clone())
    }

    fn persisted(&self) -> sync::watch::Receiver<BlockStoreState> {
        self.0.persisted.subscribe()
    }

    async fn get_validator_committee(
        &self,
        _ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
    ) -> ctx::Result<validator::Committee> {
        // For simplicity we just use the validator committee from the genesis
        // and never change it.
        Ok(self.0.genesis.validators.clone())
    }

    async fn get_pending_validator_committee(
        &self,
        _ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<(validator::Committee, validator::BlockNumber)>> {
        Ok(None)
    }

    async fn get_block(
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
            self.verify_pregenesis_block(ctx, b).await?;
        }
        let mut blocks = self.0.blocks.lock().unwrap();
        let want = self.0.persisted.borrow().next();
        if block.number() < want {
            // It may happen that a block gets fetched which is not needed any more.
            return Ok(());
        }
        if let Some(c) = self.0.capacity {
            if blocks.len() >= c {
                blocks.pop_front();
            }
        }
        if block.number() > want {
            // Blocks should be stored in order though.
            return Err(
                anyhow::format_err!("got block {:?}, want {want:?}", block.number()).into(),
            );
        }
        self.0
            .persisted
            .send_modify(|p| p.last = Some(Last::from(&block)));
        blocks.push_back(block);
        Ok(())
    }

    async fn verify_pregenesis_block(
        &self,
        _ctx: &ctx::Ctx,
        block: &validator::PreGenesisBlock,
    ) -> ctx::Result<()> {
        if self.0.pregenesis_blocks.get(&block.number) != Some(block) {
            return Err(anyhow::format_err!("invalid pre-genesis block").into());
        }
        Ok(())
    }

    async fn verify_payload(
        &self,
        _ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
        _payload: &validator::Payload,
    ) -> ctx::Result<()> {
        self.0.payload_manager.verify()
    }

    async fn propose_payload(
        &self,
        ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload> {
        self.0.payload_manager.propose(ctx).await
    }

    async fn get_state(&self, _ctx: &ctx::Ctx) -> ctx::Result<ReplicaState> {
        Ok(self.0.state.lock().unwrap().clone())
    }

    async fn set_state(&self, _ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()> {
        *self.0.state.lock().unwrap() = state.clone();
        Ok(())
    }
}

#[derive(Debug)]
struct EngineInner {
    genesis: validator::Genesis,
    persisted: sync::watch::Sender<BlockStoreState>,
    blocks: Mutex<VecDeque<validator::Block>>,
    pregenesis_blocks: HashMap<validator::BlockNumber, validator::PreGenesisBlock>,
    payload_manager: PayloadManager,
    state: Arc<Mutex<ReplicaState>>,
    capacity: Option<usize>,
}

/// Payload manager for testing purposes.
#[derive(Debug)]
pub enum PayloadManager {
    /// `propose()` creates random payloads of the given size and `verify()` accepts all payloads.
    Random(usize),
    /// `propose()` blocks indefinitely and `verify()` accepts all payloads.
    Pending,
    /// `propose()` creates empty payloads and `verify()` rejects all payloads.
    Reject,
}

impl PayloadManager {
    async fn propose(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::Payload> {
        match self {
            PayloadManager::Random(size) => {
                let mut payload = validator::Payload(vec![0; *size]);
                ctx.rng().fill(&mut payload.0[..]);
                Ok(payload)
            }
            PayloadManager::Pending => {
                ctx.canceled().await;
                Err(ctx::Canceled.into())
            }
            PayloadManager::Reject => Ok(validator::Payload(vec![])),
        }
    }

    fn verify(&self) -> ctx::Result<()> {
        match self {
            PayloadManager::Random(_) | PayloadManager::Pending => Ok(()),
            PayloadManager::Reject => Err(anyhow::anyhow!("invalid payload").into()),
        }
    }
}
