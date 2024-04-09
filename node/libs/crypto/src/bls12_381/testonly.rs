//! Random key generation, intended for use in testing

use super::{bls, AggregateSignature, PublicKey, SecretKey, Signature};
use rand::{distributions::Standard, prelude::Distribution, Rng};

/// Generates a random SecretKey. This is meant for testing purposes.
impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        SecretKey(bls::SecretKey::key_gen_v4_5(&rng.gen::<[u8; 32]>(), &[], &[]).unwrap())
    }
}

/// Generates a random Signature. This is meant for testing purposes.
impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        let key = rng.gen::<SecretKey>();
        let msg = rng.gen::<[u8; 4]>();
        key.sign(&msg)
    }
}

/// Generates a random AggregateSignature. This is meant for testing purposes.
impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        let sig: Signature = self.sample(rng);
        AggregateSignature(sig.0)
    }
}

/// Generates a random PublicKey. This is meant for testing purposes.
impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        rng.gen::<SecretKey>().public()
    }
}
