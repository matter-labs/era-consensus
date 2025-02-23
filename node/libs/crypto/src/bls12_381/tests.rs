use rand::{rngs::StdRng, Rng, SeedableRng};

use super::*;

// Represents the public key at infinity.
const INFINITY_PUBLIC_KEY: [u8; 96] = [
    0xc0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0,
];

#[test]
fn infinity_public_key_failure() {
    assert!(PublicKey::decode(&INFINITY_PUBLIC_KEY).is_err())
}

// Test signing and verifying a random message
#[test]
fn signature_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    let sk: SecretKey = rng.gen();
    let msg: [u8; 32] = rng.gen();
    let sig = sk.sign(&msg);
    sig.verify(&msg, &sk.public()).unwrap()
}

// Make sure we reject an obviously invalid signature
#[test]
fn signature_failure_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    let sk1: SecretKey = rng.gen();
    let sk2: SecretKey = rng.gen();
    let msg: [u8; 32] = rng.gen();
    let sig = sk1.sign(&msg);
    assert!(sig.verify(&msg, &sk2.public()).is_err())
}

// Test signing and verifying a proof of possession
#[test]
fn pop_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    let sk: SecretKey = rng.gen();
    let pop = sk.sign_pop();
    pop.verify(&sk.public()).unwrap()
}

// Make sure we reject an obviously invalid proof of possession
#[test]
fn pop_failure_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    let sk1: SecretKey = rng.gen();
    let sk2: SecretKey = rng.gen();

    let pop = sk1.sign_pop();
    assert!(pop.verify(&sk2.public()).is_err());

    let msg = sk1.public().encode();
    let sig = sk1.sign(&msg);
    let pop = ProofOfPossession(sig.0);
    assert!(pop.verify(&sk1.public()).is_err());
}

// Test signing and verifying a random message using aggregate signatures
#[test]
fn aggregate_signature_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = (0..5).map(|_| rng.gen()).collect();
    let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
    let msg: [u8; 32] = rng.gen();

    let sigs: Vec<Signature> = sks.iter().map(|k| k.sign(&msg)).collect();
    let agg_sig = AggregateSignature::aggregate(&sigs);

    agg_sig.verify(pks.iter().map(|pk| (&msg[..], pk))).unwrap()
}

// Make sure trying to verify a signature with too few shares fails
#[test]
fn aggregate_signature_failure_smoke() {
    let mut rng = StdRng::seed_from_u64(29483920);

    // Use an arbitrary 5 keys for the smoke test
    let sks: Vec<SecretKey> = (0..5).map(|_| rng.gen()).collect();
    let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
    let msg: [u8; 32] = rng.gen();

    // Take only three signatures for the aggregate
    let sigs: Vec<Signature> = sks.iter().take(3).map(|k| k.sign(&msg)).collect();

    let agg_sig = AggregateSignature::aggregate(&sigs);

    assert!(agg_sig.verify(pks.iter().map(|pk| (&msg[..], pk))).is_err())
}
