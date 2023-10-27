use crate::bn254_2::{AggregateSignature, PublicKey, SecretKey, Signature};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::iter::repeat_with;

#[test]
fn signature_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);
    let sk = SecretKey::random(&mut rng);
    let pk = sk.public();

    let msg: [u8; 32] = rng.gen();
    let sig = sk.sign(&msg);

    sig.verify(&msg, &pk).unwrap()
}

#[test]
fn signature_failure_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    let sk1 = SecretKey::random(&mut rng);
    let sk2 = SecretKey::random(&mut rng);
    let pk2 = sk2.public();
    let msg: [u8; 32] = rng.gen();
    let sig = sk1.sign(&msg);

    assert!(sig.verify(&msg, &pk2).is_err())
}

#[test]
fn aggregate_signature_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = repeat_with(|| SecretKey::random(&mut rng))
        .take(5)
        .collect();
    let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
    let msg: [u8; 32] = rng.gen();

    let sigs: Vec<Signature> = sks.iter().map(|k| k.sign(&msg)).collect();
    let agg_sig = AggregateSignature::aggregate(&sigs);

    agg_sig.verify(pks.iter().map(|pk| (&msg[..], pk))).unwrap()
}

#[test]
fn aggregate_signature_failure_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = repeat_with(|| SecretKey::random(&mut rng))
        .take(5)
        .collect();
    let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
    let msg: [u8; 32] = rng.gen();

    // Take only three signatures for the aggregate
    let sigs: Vec<Signature> = sks.iter().take(3).map(|k| k.sign(&msg)).collect();

    let agg_sig = AggregateSignature::aggregate(&sigs);

    assert!(agg_sig.verify(pks.iter().map(|pk| (&msg[..], pk))).is_err())
}

#[test]
fn aggregate_signature_distinct_messages() {
    let mut rng = StdRng::seed_from_u64(29483920);
    let num_keys = 5;
    let num_distinct = 2;

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = repeat_with(|| SecretKey::random(&mut rng))
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

    let agg_sig = AggregateSignature::aggregate(&sigs);

    agg_sig.verify(pairs.into_iter()).unwrap()
}
