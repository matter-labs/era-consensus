//! Random hash generation, intended for use in testing

use crate::sha256::Sha256;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};

impl Distribution<Sha256> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Sha256 {
        Sha256(rng.gen())
    }
}
