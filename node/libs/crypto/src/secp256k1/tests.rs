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

fn test_byte_format<T>(rng: &mut impl Rng)
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

fn gen_msg(rng: &mut impl Rng) -> Vec<u8> {
    let n = rng.gen_range(0..100);
    let mut msg = vec![0u8; n];
    rng.fill_bytes(&mut msg);
    msg
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

#[test]
fn prop_sign_verify() {
    let rng = &mut make_rng();

    for _ in 0..10 {
        let sk = rng.gen::<SecretKey>();
        let pk = sk.public();
        let msg = gen_msg(rng);
        let sig = sk.sign(&msg).unwrap();
        sig.verify(&msg, &pk).unwrap();
    }
}

#[test]
fn prop_sign_verify_wrong_key_fail() {
    let rng = &mut make_rng();

    for _ in 0..10 {
        let sk1 = rng.gen::<SecretKey>();
        let sk2 = rng.gen::<SecretKey>();
        let msg = gen_msg(rng);
        let sig = sk1.sign(&msg).unwrap();
        sig.verify(&msg, &sk2.public()).unwrap_err();
    }
}

#[test]
fn prop_sign_verify_wrong_msg_fail() {
    let rng = &mut make_rng();

    for _ in 0..10 {
        let sk = rng.gen::<SecretKey>();
        let msg1 = gen_msg(rng);
        let msg2 = gen_msg(rng);
        let sig = sk.sign(&msg1).unwrap();
        sig.verify(&msg2, &sk.public()).unwrap_err();
    }
}

#[test]
fn prop_sign_verify_agg() {
    let rng = &mut make_rng();

    for _ in 0..10 {
        let mut agg = AggregateSignature::default();
        let mut inputs = Vec::new();
        for _ in 0..rng.gen_range(0..5) {
            let sk = rng.gen::<SecretKey>();
            let msg = gen_msg(rng);
            let sig = sk.sign(&msg).unwrap();
            agg.add(sig);
            inputs.push((msg, sk.public()));
        }
        agg.verify(inputs.iter().map(|(msg, pk)| (msg.as_slice(), pk)))
            .unwrap();
    }
}

#[test]
fn prop_sign_verify_agg_fail() {
    let rng = &mut make_rng();

    for _ in 0..10 {
        let mut agg = AggregateSignature::default();
        let mut inputs = Vec::new();
        // Minimum two signatures
        for _ in 0..=rng.gen_range(2..5) {
            let sk = rng.gen::<SecretKey>();
            let msg = gen_msg(rng);
            let sig = sk.sign(&msg).unwrap();
            agg.add(sig);
            inputs.push((msg, sk.public()));
        }
        // Do something to mess it up.
        match rng.gen_range(0..=3) {
            0 => inputs.reverse(),
            1 => agg.0.reverse(),
            2 => {
                inputs.pop();
            }
            3 => {
                agg.0.pop();
            }
            _ => unreachable!(),
        }
        agg.verify(inputs.iter().map(|(msg, pk)| (msg.as_slice(), pk)))
            .unwrap_err();
    }
}
