//! Test-only utilities.
use crate::{Proposal, ReplicaState, PersistentBlockStore};
use rand::{distributions::Standard, prelude::Distribution, Rng};
use zksync_consensus_roles::validator;
use zksync_concurrency::ctx;

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

pub async fn dump(ctx: &ctx::Ctx, store: &dyn PersistentBlockStore) -> Vec<validator::FinalBlock> {
    let Some(range) = store.state(ctx).await.unwrap() else { return vec![] };
    let mut blocks = vec![];
    for n in range.first.header().number.0..range.next().0 {
        let n = validator::BlockNumber(n);
        let block = store.block(ctx, n).await.unwrap().unwrap();
        assert_eq!(block.header().number, n);
        blocks.push(block);
    }
    assert!(store.block(ctx, range.next()).await.unwrap().is_none());
    blocks
}

pub fn random_blocks(ctx: &ctx::Ctx) -> impl Iterator<Item=validator::FinalBlock> {
    let mut rng = ctx.rng();
    let v = validator::ProtocolVersion::EARLIEST;
    std::iter::successors(
        Some(validator::testonly::make_genesis_block(&mut rng, v)),
        move |parent| Some(validator::testonly::make_block(&mut rng, parent.header(), v)),
    )
}
