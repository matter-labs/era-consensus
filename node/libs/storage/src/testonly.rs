//! Test-only utilities.

use crate::types::ReplicaState;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use roles::validator;
use std::collections::{BTreeMap, HashMap};

impl Distribution<ReplicaState> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaState {
        let n = rng.gen_range(1..11);

        let mut block_proposal_cache: BTreeMap<_, HashMap<_, _>> = BTreeMap::new();

        for _ in 0..n {
            let block: validator::Block = rng.gen();

            match block_proposal_cache.get_mut(&block.header.number) {
                Some(blocks) => {
                    blocks.insert(block.header.hash(), block.clone());
                }
                None => {
                    let mut blocks = HashMap::new();
                    blocks.insert(block.header.hash(), block.clone());
                    block_proposal_cache.insert(block.header.number, blocks);
                }
            }
        }

        ReplicaState {
            view: rng.gen(),
            phase: rng.gen(),
            high_vote: rng.gen(),
            high_qc: rng.gen(),
            block_proposal_cache,
        }
    }
}
