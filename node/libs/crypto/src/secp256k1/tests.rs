use std::fmt::Debug;

use rand::{
    distributions::{Distribution, Standard},
    rngs::StdRng,
    seq::SliceRandom,
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

        // Verification should work for any order of messages and signatures.
        agg.0.shuffle(rng);
        inputs.shuffle(rng);

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
        // Minimum two signatures so that we can pop something.
        for _ in 0..=rng.gen_range(2..5) {
            let sk = rng.gen::<SecretKey>();
            let msg = gen_msg(rng);
            let sig = sk.sign(&msg).unwrap();
            agg.add(sig);
            inputs.push((msg, sk.public()));
        }
        // Do something to mess it up.
        if rng.gen_bool(0.5) {
            inputs.pop();
        } else {
            agg.0.pop();
        }
        agg.verify(inputs.iter().map(|(msg, pk)| (msg.as_slice(), pk)))
            .unwrap_err();
    }
}

/// Test vectors from <https://web3js.readthedocs.io/en/v1.2.0/web3-eth-accounts.html#eth-accounts-signtransaction>
///
/// The same example has been used in <https://github.com/gakonst/ethers-rs/blob/ba00f549/ethers-signers/src/wallet/private_key.rs#L197>
/// and transitively in <https://github.com/RustCrypto/elliptic-curves/blob/82e1b11fba2b01a6e60add989ab50af5cad14d5f/k256/src/ecdsa.rs#L234C14-L234C110>
///
/// This isn't strictly a use case for us, it was just a test to see what's happening with the recovery ID.
#[test]
fn test_ethereum_example() {
    let unhex = |h| hex::decode(h).unwrap();
    // Hexadecimal values from the web3js example.
    // The rawTransaction isn't used becuase it itself contains the signature and isn't what has been hashed.
    let mh = "88cfbd7e51c7a40540b233cf68b62ad1df3e92462f1c6018d6d67eae0f3b08f5";
    let sk = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    let r = "c9cf86333bcb065d140032ecaab5d9281bde80f21b9687b3e94161de42d51895";
    let s = "727a108a0b8d101465414033c3f705a9c7b826e596766046ee1183dbc8aeaa68";
    let v = "25"; // Includes the chain ID of 1

    let mh = unhex(mh);
    let sk = {
        let decoded = SecretKey::decode(&unhex(sk)).unwrap();
        assert_eq!(hex::encode(decoded.encode()), sk, "sk encodes");
        decoded
    };

    // The signature in the example should verify the hash.
    let sig = format!("{r}{s}{v}");
    let sig = Signature::decode(&unhex(&sig)).expect("sig decodes");
    sig.verify_hash(&mh, &sk.public())
        .expect("decoded sig verifies message hash");

    // Check if we calculate the same R and V.
    let sig = sk.sign_hash(&mh).unwrap();
    assert_eq!(hex::encode(sig.sig.r().to_bytes()), r, "r matches");
    assert_eq!(hex::encode(sig.sig.s().to_bytes()), s, "s matches");
    assert_eq!(sig.recid.to_byte(), 0x0, "v is not shifted");

    let bz = sig.encode();
    assert!(bz[64] == 27 || bz[64] == 28, "v is shifted when encoded");
}
