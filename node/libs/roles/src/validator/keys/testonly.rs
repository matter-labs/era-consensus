//! Test-only utilities.
use super::{AggregateSignature, ProofOfPossession, PublicKey, SecretKey, Signature};
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;

impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        AggregateSignature(rng.gen())
    }
}

impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        PublicKey(rng.gen())
    }
}

impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        Signature(rng.gen())
    }
}

impl Distribution<ProofOfPossession> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ProofOfPossession {
        ProofOfPossession(rng.gen())
    }
}

impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        SecretKey(Arc::new(rng.gen()))
    }
}
