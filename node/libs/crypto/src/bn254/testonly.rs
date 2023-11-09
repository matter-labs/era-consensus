//! Random key generation, intended for use in testing

use super::{AggregateSignature, PublicKey, SecretKey, Signature};
use rand::{distributions::Standard, prelude::Distribution, Rng};

/// Generates a random SecretKey. This is meant for testing purposes.
impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        let rand = ark_bn254::Fr::new(rng.gen());
        SecretKey(rand)
    }
}

/// Generates a random PublicKey. This is meant for testing purposes.
impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        PublicKey(rng.gen())
    }
}

/// Generates a random Signature. This is meant for testing purposes.
impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        Signature(rng.gen())
    }
}

/// Generates a random AggregateSignature. This is meant for testing purposes.
impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        AggregateSignature(rng.gen())
    }
}
