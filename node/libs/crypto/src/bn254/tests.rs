use crate::bn254::{AggregateSignature, PublicKey, SecretKey, Signature};
use crate::ByteFmt;
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
