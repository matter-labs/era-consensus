//! Test-only utilities.
use crate::{PersistentBlockStore, BlockStore, BlockStoreRunner, Proposal, ReplicaState};
use rand::{distributions::Standard, prelude::Distribution, Rng};
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;
use std::sync::Arc;

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
pub async fn new_store(ctx: &ctx::Ctx, genesis: &validator::FinalBlock) -> (Arc<BlockStore>,BlockStoreRunner) {
    BlockStore::new(ctx,Box::new(in_memory::BlockStore::new(genesis.clone()))).await.unwrap()
}

/// Dumps all the blocks stored in `store`.
pub async fn dump(ctx: &ctx::Ctx, store: &dyn PersistentBlockStore) -> Vec<validator::FinalBlock> {
    let range = store.state(ctx).await.unwrap();
    let mut blocks = vec![];
    for n in range.first.header().number.0..range.next().0 {
        let n = validator::BlockNumber(n);
        let block = store.block(ctx, n).await.unwrap();
        assert_eq!(block.header().number, n);
        blocks.push(block);
    }
    assert!(store.block(ctx, range.next()).await.is_err());
    blocks
}
