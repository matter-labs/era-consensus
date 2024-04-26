use crate::{
    bn254::{AggregateSignature, PublicKey, SecretKey, Signature},
    ByteFmt,
};
use ff_ce::{Field, PrimeField};
use num_bigint::BigUint;
use pairing::{
    bn256::{Fr, G2Affine, G2},
    CurveAffine, CurveProjective,
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::iter::repeat_with;

#[test]
fn signature_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);
    let sk = rng.gen::<SecretKey>();
    let pk = sk.public();

    let msg = rng.gen::<[u8; 32]>();
    let sig = sk.sign(&msg);

    sig.verify(&msg, &pk).unwrap()
}

#[test]
fn test_secret_key_generate() {
    let mut rng = rand::thread_rng();
    let sk = rng.gen::<SecretKey>();
    // assert 1 <= sk < r
    assert!(sk.0.into_repr() < Fr::char());
    assert!(!sk.0.is_zero());
}

#[test]
fn test_decode_zero_secret_key_failure() {
    let mut bytes: [u8; 1000] = [0; 1000];
    bytes[0] = 1;
    bytes[800] = 1;
    SecretKey::decode(&bytes).expect_err("Oversized secret key decoded");

    let mut bytes: [u8; 33] = [0; 33];
    bytes[0] = 1;
    bytes[32] = 1;
    SecretKey::decode(&bytes).expect_err("Oversized 33 bytes secret key decoded");

    let bytes: [u8; 31] = [0; 31];
    SecretKey::decode(&bytes).expect_err("Undersized secret key decoded");

    let bytes: [u8; 32] = [0; 32];
    SecretKey::decode(&bytes).expect_err("zero secret key decoded");

    // r is taken from https://hackmd.io/@jpw/bn254#Parameter-for-BN254
    let r_decimal_str =
        "21888242871839275222246405745257275088548364400416034343698204186575808495617";
    let decimal_biguint = BigUint::parse_bytes(r_decimal_str.as_bytes(), 10).unwrap();
    let bytes = decimal_biguint.to_bytes_be();
    SecretKey::decode(&bytes).expect_err("sk >= r decoded");
}

#[test]
fn signature_failure_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    let sk1 = rng.gen::<SecretKey>();
    let sk2 = rng.gen::<SecretKey>();
    let pk2 = sk2.public();
    let msg = rng.gen::<[u8; 32]>();
    let sig = sk1.sign(&msg);

    assert!(sig.verify(&msg, &pk2).is_err())
}

#[test]
fn aggregate_signature_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = repeat_with(|| rng.gen::<SecretKey>()).take(5).collect();
    let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
    let msg = rng.gen::<[u8; 32]>();

    let sigs: Vec<Signature> = sks.iter().map(|k| k.sign(&msg)).collect();
    let agg = AggregateSignature::aggregate(&sigs);

    agg.verify(pks.iter().map(|pk| (&msg[..], pk))).unwrap()
}

#[test]
fn aggregate_signature_distinct_messages() {
    let mut rng = StdRng::seed_from_u64(29483920);
    let num_keys = 5;
    let num_distinct = 2;

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = repeat_with(|| rng.gen::<SecretKey>())
        .take(num_keys)
        .collect();
    let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
    // Create 2 distinct messages
    let msgs: Vec<[u8; 32]> = repeat_with(|| rng.gen()).take(num_distinct).collect();

    let mut sigs: Vec<Signature> = Vec::new();
    let mut pairs: Vec<(&[u8], &PublicKey)> = Vec::new();
    for (i, sk) in sks.iter().enumerate() {
        let msg = &msgs[i % num_distinct];
        sigs.push(sk.sign(msg));
        pairs.push((msg, &pks[i]))
    }

    let agg = AggregateSignature::aggregate(&sigs);

    agg.verify(pairs.into_iter()).unwrap()
}

#[test]
fn aggregate_signature_failure_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = repeat_with(|| rng.gen::<SecretKey>()).take(5).collect();
    let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
    let msg = rng.gen::<[u8; 32]>();

    // Take only three signatures for the aggregate
    let sigs: Vec<Signature> = sks.iter().take(3).map(|k| k.sign(&msg)).collect();

    let agg = AggregateSignature::aggregate(&sigs);

    assert!(agg.verify(pks.iter().map(|pk| (&msg[..], pk))).is_err())
}

#[test]
fn byte_fmt_correctness() {
    let mut rng = rand::thread_rng();

    let sk: SecretKey = rng.gen();
    let bytes = sk.encode();
    let sk_decoded = SecretKey::decode(&bytes).unwrap();
    assert_eq!(sk, sk_decoded);

    let pk: PublicKey = rng.gen();
    let bytes = pk.encode();
    let pk_decoded = PublicKey::decode(&bytes).unwrap();
    assert_eq!(pk, pk_decoded);

    let sig: Signature = rng.gen();
    let bytes = sig.encode();
    let sig_decoded = Signature::decode(&bytes).unwrap();
    assert_eq!(sig, sig_decoded);

    let agg: AggregateSignature = rng.gen();
    let bytes = agg.encode();
    let agg_decoded = AggregateSignature::decode(&bytes).unwrap();
    assert_eq!(agg, agg_decoded);
}

#[test]
fn pk_is_valid_correctness() {
    // Check that the null point is invalid.
    let mut pk = PublicKey(G2::zero());
    assert!(pk.verify().is_err());

    // Check that a point in the wrong subgroup is invalid.
    let mut rng = rand04::OsRng::new().unwrap();

    loop {
        let x = rand04::Rand::rand(&mut rng);
        let greatest = rand04::Rand::rand(&mut rng);

        if let Some(p) = G2Affine::get_point_from_x(x, greatest) {
            if !p.is_zero() && p.is_on_curve() {
                // Check that it's not on the subgroup.
                let order = Fr::char();
                let mut p = p.into_projective();
                p.mul_assign(order);

                if !p.is_zero() {
                    pk = PublicKey(p);
                    break;
                }
            }
        }
    }

    assert!(pk.verify().is_err());
}
