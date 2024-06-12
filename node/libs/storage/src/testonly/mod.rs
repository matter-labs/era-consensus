//! Test-only utilities.
use crate::{
    store::{BlockStore, StoreRunner},
    PersistentStore, Proposal, ReplicaState,
};
use anyhow::Context as _;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use std::sync::Arc;
use zksync_concurrency::ctx;
use zksync_consensus_roles::{attester, validator};

pub mod in_memory;

impl Distribution<Proposal> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Proposal {
        Proposal {
            number: rng.gen(),
            payload: rng.gen(),
        }
    }
}

impl Distribution<ReplicaState> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaState {
        ReplicaState {
            view: rng.gen(),
            phase: rng.gen(),
            high_vote: rng.gen(),
            high_qc: rng.gen(),
            proposals: (0..rng.gen_range(1..11)).map(|_| rng.gen()).collect(),
        }
    }
}

/// Test-only memory storage for blocks and batches.
pub struct TestMemoryStorage {
    /// In-memory block store with its runner.
    pub blocks: Arc<BlockStore>,
    /// In-memory storage runner.
    pub runner: StoreRunner,
}

impl TestMemoryStorage {
    /// Constructs a new in-memory store for both blocks and batches with their respective runners.
    pub async fn new(ctx: &ctx::Ctx, genesis: &validator::Genesis) -> Self {
        let (store, runner) = new_store(ctx, genesis).await;
        Self {
            blocks: store,
            runner,
        }
    }

    /// Constructs a new in-memory store with a custom expected first block
    /// (i.e. possibly different than `genesis.fork.first_block`).
    pub async fn new_store_with_first_block(
        ctx: &ctx::Ctx,
        genesis: &validator::Genesis,
        first: validator::BlockNumber,
    ) -> Self {
        let (store, runner) = new_store_with_first(ctx, genesis, first).await;
        Self {
            blocks: store,
            runner,
        }
    }
}

/// Constructs a new in-memory store.
async fn new_store(ctx: &ctx::Ctx, genesis: &validator::Genesis) -> (Arc<BlockStore>, StoreRunner) {
    new_store_with_first(ctx, genesis, genesis.first_block).await
}

/// Constructs a new in-memory store with a custom expected first block
/// (i.e. possibly different than `genesis.fork.first_block`).
async fn new_store_with_first(
    ctx: &ctx::Ctx,
    genesis: &validator::Genesis,
    first: validator::BlockNumber,
) -> (Arc<BlockStore>, StoreRunner) {
    BlockStore::new(
        ctx,
        Box::new(in_memory::BlockStore::new(genesis.clone(), first)),
    )
    .await
    .unwrap()
}

/// Dumps all the batches stored in `store`.
pub async fn dump(ctx: &ctx::Ctx, store: &dyn PersistentStore) -> Vec<validator::FinalBlock> {
    // let genesis = store.genesis(ctx).await.unwrap();
    let state = store.persisted().borrow().clone();
    // assert!(genesis.first_block <= state.first);
    let mut blocks = vec![];
    let after = state
        .1
        .last
        .as_ref()
        .map(|qc| qc.header().number.next())
        .unwrap_or(state.1.first);
    for n in (state.1.first.0..after.0).map(validator::BlockNumber) {
        let block = store.block(ctx, n).await.unwrap();
        assert_eq!(block.header().number, n);
        blocks.push(block);
    }
    if let Some(before) = state.1.first.prev() {
        assert!(store.block(ctx, before).await.is_err());
    }
    assert!(store.block(ctx, after).await.is_err());
    blocks
}

/// Dumps all the batches stored in `store`.
pub async fn dump_batch(_ctx: &ctx::Ctx, store: &dyn PersistentStore) -> Vec<attester::SyncBatch> {
    // let genesis = store.genesis(ctx).await.unwrap();
    let state = store.persisted().borrow().clone();
    // assert!(genesis.first_block <= state.first);
    let mut batches = vec![];
    let after = state
        .0
        .last
        .as_ref()
        .map(|sb| sb.number.next())
        .unwrap_or(state.0.first);
    for n in (state.0.first.0..after.0).map(attester::BatchNumber) {
        if n.0 == 0 {
            continue;
        }

        let batch = store.batch(n).await.unwrap();
        assert_eq!(batch.number, n);
        batches.push(batch);
    }
    if let Some(before) = state.0.first.prev() {
        assert!(store.batch(before).await.is_err());
    }
    assert!(store.batch(after).await.is_err());
    batches
}

/// Verifies storage content.
pub async fn verify(ctx: &ctx::Ctx, store: &BlockStore) -> anyhow::Result<()> {
    let range = store.queued().1;
    for n in (range.first.0..range.next().0).map(validator::BlockNumber) {
        async {
            store
                .block(ctx, n)
                .await?
                .context("missing")?
                .verify(store.genesis())
                .context("verify()")
        }
        .await
        .context(n)?;
    }
    Ok(())
}
