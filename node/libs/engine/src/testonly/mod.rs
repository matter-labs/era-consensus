//! Test-only utilities.
use std::sync::Arc;

use in_memory::PayloadManager;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use zksync_concurrency::{ctx, time};
use zksync_consensus_roles::{validator, validator::testonly::Setup};

use crate::{
    BlockStoreState, EngineInterface, EngineManager, EngineManagerRunner, Last, Transaction,
};

pub mod in_memory;

impl Distribution<Last> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Last {
        match rng.gen_range(0..2) {
            0 => Last::PreGenesis(rng.gen()),
            1 => Last::FinalV2(rng.gen()),
            _ => unreachable!(),
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

impl Distribution<Transaction> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Transaction {
        let size: usize = rng.gen_range(10..100);
        Transaction((0..size).map(|_| rng.gen()).collect())
    }
}

/// Test-only engine.
pub struct TestEngine {
    /// In-memory engine manager.
    pub manager: Arc<EngineManager>,
    /// In-memory engine manager runner.
    pub runner: EngineManagerRunner,
    /// The in-memory engine representing the execution layer.
    pub im_engine: in_memory::Engine,
}

impl TestEngine {
    /// Constructs a new in-memory engine manager with the given setup.
    pub async fn new(ctx: &ctx::Ctx, setup: &Setup) -> Self {
        Self::new_with_first_block(ctx, setup, setup.first_block()).await
    }

    /// Constructs a new in-memory engine manager with a custom expected first block
    /// (i.e. possibly different than `genesis.fork.first_block`).
    pub async fn new_with_first_block(
        ctx: &ctx::Ctx,
        setup: &Setup,
        first: validator::BlockNumber,
    ) -> Self {
        let im_engine = in_memory::Engine::new_random(setup, first);
        let (engine, runner) =
            EngineManager::new(ctx, Box::new(im_engine.clone()), time::Duration::seconds(1))
                .await
                .unwrap();
        Self {
            manager: engine,
            runner,
            im_engine,
        }
    }

    /// Constructs a new in-memory engine manager with a custom payload manager.
    pub async fn new_with_payload_manager(
        ctx: &ctx::Ctx,
        setup: &Setup,
        payload_manager: PayloadManager,
    ) -> Self {
        let im_engine = in_memory::Engine::new(setup, setup.first_block(), payload_manager);
        let (engine, runner) =
            EngineManager::new(ctx, Box::new(im_engine.clone()), time::Duration::seconds(1))
                .await
                .unwrap();
        Self {
            manager: engine,
            runner,
            im_engine,
        }
    }

    /// Constructs a new in-memory engine manager with a dynamic validator schedule.
    pub async fn new_with_dynamic_schedule(ctx: &ctx::Ctx, setup: &Setup, n: u64) -> Self {
        // We can only have dynamic validator schedules if we don't have a static one in genesis.
        assert!(setup.genesis.validators_schedule.is_none());

        let im_engine = in_memory::Engine::new_dynamic_schedule(setup, setup.first_block(), n);
        let (engine, runner) =
            EngineManager::new(ctx, Box::new(im_engine.clone()), time::Duration::seconds(1))
                .await
                .unwrap();
        Self {
            manager: engine,
            runner,
            im_engine,
        }
    }
}

/// Dumps all the blocks stored in `block_store` of an `EngineManager`.
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
