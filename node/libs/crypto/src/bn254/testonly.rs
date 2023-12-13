//! Random key generation, intended for use in testing

use super::{AggregateSignature, PublicKey, SecretKey, Signature};
use pairing::bn256::{Fr, G1, G2};
use rand::{distributions::Standard, prelude::Distribution, Rng, RngCore};
use rand04::Rand;

struct RngWrapper<R>(R);

impl<R: RngCore> rand04::Rng for RngWrapper<R> {
    fn next_u32(&mut self) -> u32 {
        self.0.next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }
}

/// Generates a random SecretKey. This is meant for testing purposes.
impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        let scalar = Fr::rand(&mut RngWrapper(rng));
        SecretKey(scalar)
    }
}

/// Generates a random PublicKey. This is meant for testing purposes.
impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        let p = G2::rand(&mut RngWrapper(rng));
        PublicKey(p)
    }
}

/// Generates a random Signature. This is meant for testing purposes.
impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        let p = G1::rand(&mut RngWrapper(rng));
        Signature(p)
    }
}

/// Generates a random AggregateSignature. This is meant for testing purposes.
impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        let p = G1::rand(&mut RngWrapper(rng));
        AggregateSignature(p)
    }
}

impl AggregateSignature {
    /// Generate a new aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        let mut agg = Self::default();
        for sig in sigs {
            agg.add(sig);
        }
        agg
    }
}
