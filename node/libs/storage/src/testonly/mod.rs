//! Test-only utilities.
use crate::{
    batch_store::BatchStoreRunner, BatchStore, BlockStore, BlockStoreRunner, PersistentBatchStore,
    PersistentBlockStore, Proposal, ReplicaState,
};
use anyhow::Context as _;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use std::sync::Arc;
use zksync_concurrency::ctx;
use zksync_consensus_roles::{
    attester::{self, BatchNumber},
    validator,
};

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
    pub blocks: (Arc<BlockStore>, BlockStoreRunner),
    /// In-memory batch store with its runner.
    pub batches: (Arc<BatchStore>, BatchStoreRunner),
}

impl TestMemoryStorage {
    /// Constructs a new in-memory store for both blocks and batches with their respective runners.
    pub async fn new(ctx: &ctx::Ctx, genesis: &validator::Genesis) -> Self {
        let (blocks, blocks_runner) = new_store(ctx, &genesis).await;
        let (batches, batches_runner) = new_batch_store(ctx, &genesis).await;
        Self {
            blocks: (blocks, blocks_runner),
            batches: (batches, batches_runner),
        }
    }

    /// Constructs a new in-memory store with a custom expected first block
    /// (i.e. possibly different than `genesis.fork.first_block`).
    pub async fn new_store_with_first_block(
        ctx: &ctx::Ctx,
        genesis: &validator::Genesis,
        first: validator::BlockNumber,
    ) -> Self {
        let (blocks, blocks_runner) = new_store_with_first(ctx, genesis, first).await;
        let (batches, batches_runner) = new_batch_store(ctx, genesis).await;
        Self {
            blocks: (blocks, blocks_runner),
            batches: (batches, batches_runner),
        }
    }
}

/// Constructs a new in-memory store.
async fn new_store(
    ctx: &ctx::Ctx,
    genesis: &validator::Genesis,
) -> (Arc<BlockStore>, BlockStoreRunner) {
    new_store_with_first(ctx, genesis, genesis.first_block).await
}

/// Constructs a new in-memory batch store.
async fn new_batch_store(
    ctx: &ctx::Ctx,
    genesis: &validator::Genesis,
) -> (Arc<BatchStore>, BatchStoreRunner) {
    BatchStore::new(
        ctx,
        Box::new(in_memory::BatchStore::new(BatchNumber(0))),
        genesis.clone(),
    )
    .await
    .unwrap()
}

/// Constructs a new in-memory store with a custom expected first block
/// (i.e. possibly different than `genesis.fork.first_block`).
async fn new_store_with_first(
    ctx: &ctx::Ctx,
    genesis: &validator::Genesis,
    first: validator::BlockNumber,
) -> (Arc<BlockStore>, BlockStoreRunner) {
    BlockStore::new(
        ctx,
        Box::new(in_memory::BlockStore::new(genesis.clone(), first)),
    )
    .await
    .unwrap()
}

/// Dumps all the blocks stored in `store`.
pub async fn dump(ctx: &ctx::Ctx, store: &dyn PersistentBlockStore) -> Vec<validator::FinalBlock> {
    let genesis = store.genesis(ctx).await.unwrap();
    let state = store.persisted().borrow().clone();
    assert!(genesis.first_block <= state.first);
    let mut blocks = vec![];
    let after = state
        .last
        .as_ref()
        .map(|qc| qc.header().number.next())
        .unwrap_or(state.first);
    for n in (state.first.0..after.0).map(validator::BlockNumber) {
        let block = store.block(ctx, n).await.unwrap();
        assert_eq!(block.header().number, n);
        blocks.push(block);
    }
    if let Some(before) = state.first.prev() {
        assert!(store.block(ctx, before).await.is_err());
    }
    assert!(store.block(ctx, after).await.is_err());
    blocks
}

/// Dumps all the batches stored in `store`.
pub async fn dump_batch(
    _ctx: &ctx::Ctx,
    store: &dyn PersistentBatchStore,
) -> Vec<attester::FinalBatch> {
    // let genesis = store.genesis(ctx).await.unwrap();
    let state = store.persisted().borrow().clone();
    // assert!(genesis.first_block <= state.first);
    let mut batches = vec![];
    let after = state
        .last
        .as_ref()
        .map(|qc| qc.header().number.next())
        .unwrap_or(state.first);
    for n in (state.first.0..after.0).map(attester::BatchNumber) {
        let batch = store.get_batch(n).unwrap();
        assert_eq!(batch.header().number, n);
        batches.push(batch);
    }
    if let Some(before) = state.first.prev() {
        assert!(store.get_batch(before).is_none());
    }
    assert!(store.get_batch(after).is_none());
    batches
}

/// Verifies storage content.
pub async fn verify(ctx: &ctx::Ctx, store: &BlockStore) -> anyhow::Result<()> {
    let range = store.queued();
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
