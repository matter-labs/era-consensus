//! Random key generation, intended for use in testing

use pairing::bn256::{Fr, G1, G2};
use rand::{distributions::Standard, prelude::Distribution, Rng};
use rand04::{thread_rng, Rand};

use super::{AggregateSignature, PublicKey, SecretKey, Signature};

/// Generates a random SecretKey. This is meant for testing purposes.
impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> SecretKey {
        // TODO: find a solution for rand::Rng trait object incompatibility with rand04::Rng trait bound
        let mut rng = thread_rng();

        let scalar = Fr::rand(&mut rng);
        SecretKey(scalar)
    }
}

/// Generates a random PublicKey. This is meant for testing purposes.
impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> PublicKey {
        // TODO: find a solution for rand::Rng trait object incompatibility with rand04::Rng trait bound
        let mut rng = thread_rng();

        let p = G2::rand(&mut rng);
        PublicKey(p)
    }
}

/// Generates a random Signature. This is meant for testing purposes.
impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> Signature {
        // TODO: find a solution for rand::Rng trait object incompatibility with rand04::Rng trait bound
        let mut rng = thread_rng();

        let p = G1::rand(&mut rng);
        Signature(p)
    }
}

/// Generates a random AggregateSignature. This is meant for testing purposes.
impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> AggregateSignature {
        // TODO: find a solution for rand::Rng trait object incompatibility with rand04::Rng trait bound
        let mut rng = thread_rng();

        let p = G1::rand(&mut rng);
        AggregateSignature(p)
    }
}
