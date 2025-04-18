//! Test-only utilities.
use std::sync::Arc;

use anyhow::Context as _;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use zksync_concurrency::ctx;
use zksync_consensus_roles::{validator, validator::testonly::Setup};

use crate::{
    block_store::BlockStore, BlockStoreState, EngineInterface, EngineManager, EngineManagerRunner,
    Last,
};

pub mod in_memory;

impl Distribution<Last> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Last {
        match rng.gen_range(0..2) {
            0 => Last::PreGenesis(rng.gen()),
            _ => Last::FinalV1(rng.gen()),
        }
    }
}

impl Distribution<BlockStoreState> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BlockStoreState {
        BlockStoreState {
            first: rng.gen(),
            last: rng.gen(),
        }
    }
}

/// Test-only engine manager for blocks.
pub struct TestEngineManager {
    /// In-memory engine manager.
    pub engine: Arc<EngineManager>,
    /// In-memory engine manager runner.
    pub runner: EngineManagerRunner,
    /// The in-memory engine representing the persistent store.
    pub im_engine: in_memory::Engine,
}

impl TestEngineManager {
    /// Constructs a new in-memory store for both blocks with their respective runners.
    pub async fn new(ctx: &ctx::Ctx, setup: &Setup) -> Self {
        Self::new_store_with_first_block(ctx, setup, setup.genesis.first_block).await
    }

    /// Constructs a new in-memory store with a custom expected first block
    /// (i.e. possibly different than `genesis.fork.first_block`).
    pub async fn new_store_with_first_block(
        ctx: &ctx::Ctx,
        setup: &Setup,
        first: validator::BlockNumber,
    ) -> Self {
        let im_blocks = in_memory::Engine::new(setup, first);
        Self::new_with_im(ctx, im_blocks).await
    }

    /// Constructs a new in-memory store for both blocks with their respective runners.
    async fn new_with_im(ctx: &ctx::Ctx, im_blocks: in_memory::Engine) -> Self {
        let (blocks, runner) = BlockStore::new(ctx, Box::new(im_blocks.clone()))
            .await
            .unwrap();
        Self {
            blocks,
            runner,
            im_blocks,
        }
    }
}

/// Dumps all the blocks stored in `store`.
pub async fn dump(ctx: &ctx::Ctx, interface: &dyn EngineInterface) -> Vec<validator::Block> {
    let state = interface.persisted().borrow().clone();
    let mut blocks = vec![];
    let after = state
        .last
        .as_ref()
        .map(|last| last.number().next())
        .unwrap_or(state.first);
    for n in (state.first.0..after.0).map(validator::BlockNumber) {
        let block = interface.get_block(ctx, n).await.unwrap();
        assert_eq!(block.number(), n);
        blocks.push(block);
    }
    if let Some(before) = state.first.prev() {
        assert!(interface.get_block(ctx, before).await.is_err());
    }
    assert!(interface.get_block(ctx, after).await.is_err());
    blocks
}

/// Verifies storage content.
pub async fn verify(ctx: &ctx::Ctx, store: &BlockStore) -> anyhow::Result<()> {
    let range = store.queued();
    for n in (range.first.0..range.next().0).map(validator::BlockNumber) {
        async {
            let b = store.block(ctx, n).await?.context("missing")?;
            store.verify_block(ctx, &b).await.context("verify_block()")
        }
        .await
        .context(n)?;
    }
    Ok(())
}
