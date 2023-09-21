use super::*;
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::iter::repeat_with;

// Test signing and verifying a random message
#[test]
fn signature_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    let sk = SecretKey::generate(rng.gen());
    let pk = sk.public();
    let msg: [u8; 32] = rng.gen();

    let sig = sk.sign(&msg);

    sig.verify(&msg, &pk).unwrap()
}

// Make sure we reject an obviously invalid signature
#[test]
fn signature_failure_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    let sk1 = SecretKey::generate(rng.gen());
    let sk2 = SecretKey::generate(rng.gen());
    let pk2 = sk2.public();
    let msg: [u8; 32] = rng.gen();

    let sig = sk1.sign(&msg);

    assert!(sig.verify(&msg, &pk2).is_err())
}

// Test signing and verifying a random message using aggregate signatures
#[test]
fn aggregate_signature_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = repeat_with(|| SecretKey::generate(rng.gen()))
        .take(5)
        .collect();
    let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
    let msg: [u8; 32] = rng.gen();

    let sigs: Vec<Signature> = sks.iter().map(|k| k.sign(&msg)).collect();
    let agg_sig = AggregateSignature::aggregate(&sigs).unwrap();

    agg_sig.verify(pks.iter().map(|pk| (&msg[..], pk))).unwrap()
}

// Make sure trying to verify a signature with too few shares fails
#[test]
fn aggregate_signature_failure_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = repeat_with(|| SecretKey::generate(rng.gen()))
        .take(5)
        .collect();
    let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
    let msg: [u8; 32] = rng.gen();

    // Take only three signatures for the aggregate
    let sigs: Vec<Signature> = sks.iter().take(3).map(|k| k.sign(&msg)).collect();

    let agg_sig = AggregateSignature::aggregate(&sigs).unwrap();

    assert!(agg_sig.verify(pks.iter().map(|pk| (&msg[..], pk))).is_err())
}
