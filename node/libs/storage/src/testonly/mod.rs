//! Test-only utilities.
use crate::{BlockStore, BlockStoreRunner, PersistentBlockStore, Proposal, ReplicaState};
use anyhow::Context as _;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use std::sync::Arc;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;

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

/// Constructs a new in-memory store with a genesis block.
pub async fn new_store(
    ctx: &ctx::Ctx,
    genesis: &validator::Genesis,
) -> (Arc<BlockStore>, BlockStoreRunner) {
    BlockStore::new(ctx, Box::new(in_memory::BlockStore::new(genesis.clone())))
        .await
        .unwrap()
}

/// Dumps all the blocks stored in `store`.
pub async fn dump(ctx: &ctx::Ctx, store: &dyn PersistentBlockStore) -> Vec<validator::FinalBlock> {
    let genesis = store.genesis(ctx).await.unwrap();
    let state = store.state(ctx).await.unwrap();
    assert!(genesis.fork.first_block <= state.first);
    let mut blocks = vec![];
    let after = state.last
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

/// Verifies storage content.
pub async fn verify(ctx: &ctx::Ctx, store: &BlockStore) -> anyhow::Result<()> {
    let range = store.subscribe().borrow().clone();
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
