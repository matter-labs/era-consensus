//! Test-only utilities.
use crate::{Proposal, ReplicaState};
use rand::{distributions::Standard, prelude::Distribution, Rng};
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
