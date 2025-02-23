//! Random key generation, intended for use in testing

use ed25519_dalek as ed;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};

use super::{PublicKey, SecretKey, Signature};
use crate::ByteFmt;

impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        let raw: [u8; ed::SECRET_KEY_LENGTH] = rng.gen();
        ByteFmt::decode(&raw).unwrap()
    }
}

impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        rng.gen::<SecretKey>().public()
    }
}

impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        let key = rng.gen::<SecretKey>();
        let msg = rng.gen::<[u8; 4]>();
        key.sign(&msg)
    }
}
