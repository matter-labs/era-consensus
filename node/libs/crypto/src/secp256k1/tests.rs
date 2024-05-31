use std::fmt::Debug;

use rand::{
    distributions::{Distribution, Standard},
    rngs::StdRng,
    Rng, SeedableRng,
};

use crate::{secp256k1::*, ByteFmt};

fn make_rng() -> StdRng {
    StdRng::seed_from_u64(29483920)
}

fn test_byte_format<T: ByteFmt>(rng: &mut impl Rng)
where
    T: ByteFmt + Eq + Debug,
    Standard: Distribution<T>,
{
    let v0 = rng.gen::<T>();
    let bz0 = v0.encode();
    let v1 = T::decode(&bz0).unwrap();
    assert_eq!(v0, v1);
    let bz1 = v1.encode();
    assert_eq!(bz0, bz1);
}

fn prop_byte_format<T>()
where
    T: ByteFmt + Eq + Debug,
    Standard: Distribution<T>,
{
    let rng = &mut make_rng();
    for _ in 0..10 {
        test_byte_format::<T>(rng);
    }
}

#[test]
fn prop_public_key_format() {
    prop_byte_format::<PublicKey>();
}

#[test]
fn prop_secret_key_format() {
    prop_byte_format::<SecretKey>();
}

#[test]
fn prop_sig_format() {
    prop_byte_format::<Signature>();
}

#[test]
fn prop_aggsig_format() {
    prop_byte_format::<AggregateSignature>();
}
