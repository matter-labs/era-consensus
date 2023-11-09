//! Random key generation, intended for use in testing

use pairing::bn256::{Fr, G1, G2};
use rand::{distributions::Standard, prelude::Distribution, Rng};
use rand04::SeedableRng;
use rand04::{Rand, StdRng as rand04StdRng};

use super::{AggregateSignature, PublicKey, SecretKey, Signature};

/// Generates a random SecretKey. This is meant for testing purposes.
impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        let mut rng = rng08_to_rng04(rng);
        let scalar = Fr::rand(&mut rng);
        SecretKey(scalar)
    }
}

/// Generates a random PublicKey. This is meant for testing purposes.
impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        let mut rng = rng08_to_rng04(rng);
        let p = G2::rand(&mut rng);
        PublicKey(p)
    }
}

/// Generates a random Signature. This is meant for testing purposes.
impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        let mut rng = rng08_to_rng04(rng);
        let p = G1::rand(&mut rng);
        Signature(p)
    }
}

/// Generates a random AggregateSignature. This is meant for testing purposes.
impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        let mut rng = rng08_to_rng04(rng);
        let p = G1::rand(&mut rng);
        AggregateSignature(p)
    }
}

fn rng08_to_rng04<R: Rng + ?Sized>(rng: &mut R) -> rand04StdRng {
    let val = rng.gen::<usize>();
    let seed: &[usize] = std::slice::from_ref(&val);
    rand04::StdRng::from_seed(seed)
}
