//! Random key generation, intended for use in testing

use super::{SecretKey, Signature};
use crate::ByteFmt;
use ed25519_dalek as ed;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};

/// Generates a random SecretKey. This is meant for testing purposes.
impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        let raw: [u8; ed::SECRET_KEY_LENGTH] = rng.gen();
        ByteFmt::decode(&raw).unwrap()
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
