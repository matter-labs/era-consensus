//! Random hash generation, intended for use in testing

use crate::keccak256::Keccak256;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};

impl Distribution<Keccak256> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Keccak256 {
        Keccak256(rng.gen())
    }
}
