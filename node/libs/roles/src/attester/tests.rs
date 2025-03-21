use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{bls12_381, ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::test_encode_random;

use super::*;

#[test]
fn test_byte_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let sk: SecretKey = rng.gen();
    assert_eq!(sk, ByteFmt::decode(&ByteFmt::encode(&sk)).unwrap());

    let pk: PublicKey = rng.gen();
    assert_eq!(pk, ByteFmt::decode(&ByteFmt::encode(&pk)).unwrap());

    let sig: Signature = rng.gen();
    assert_eq!(sig, ByteFmt::decode(&ByteFmt::encode(&sig)).unwrap());

    let agg_sig: AggregateSignature = rng.gen();
    assert_eq!(
        agg_sig,
        ByteFmt::decode(&ByteFmt::encode(&agg_sig)).unwrap()
    );
}

#[test]
fn test_text_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let sk: SecretKey = rng.gen();
    let t = TextFmt::encode(&sk);
    assert_eq!(sk, Text::new(&t).decode::<SecretKey>().unwrap());

    let pk: PublicKey = rng.gen();
    let t = TextFmt::encode(&pk);
    assert_eq!(pk, Text::new(&t).decode::<PublicKey>().unwrap());

    let sig: Signature = rng.gen();
    let t = TextFmt::encode(&sig);
    assert_eq!(sig, Text::new(&t).decode::<Signature>().unwrap());

    let msg_hash: MsgHash = rng.gen();
    let t = TextFmt::encode(&msg_hash);
    assert_eq!(msg_hash, Text::new(&t).decode::<MsgHash>().unwrap());

    let agg_sig: AggregateSignature = rng.gen();
    let t = TextFmt::encode(&agg_sig);
    assert_eq!(
        agg_sig,
        Text::new(&t).decode::<AggregateSignature>().unwrap()
    );
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<Signed<Batch>>(rng);
    test_encode_random::<BatchQC>(rng);
    test_encode_random::<Msg>(rng);
    test_encode_random::<MsgHash>(rng);
    test_encode_random::<Signers>(rng);
    test_encode_random::<PublicKey>(rng);
    test_encode_random::<Signature>(rng);
    test_encode_random::<AggregateSignature>(rng);
}

#[test]
fn test_signature_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let msg1: MsgHash = rng.gen();
    let msg2: MsgHash = rng.gen();

    let key1: SecretKey = rng.gen();
    let key2: SecretKey = rng.gen();

    let sig1 = key1.sign_hash(&msg1);

    // Matching key and message.
    sig1.verify_hash(&msg1, &key1.public()).unwrap();

    // Mismatching message.
    assert!(sig1.verify_hash(&msg2, &key1.public()).is_err());

    // Mismatching key.
    assert!(sig1.verify_hash(&msg1, &key2.public()).is_err());
}

#[test]
fn test_agg_signature_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let msg1: MsgHash = rng.gen();
    let msg2: MsgHash = rng.gen();

    let key1: bls12_381::SecretKey = rng.gen();
    let key2: bls12_381::SecretKey = rng.gen();
    let key3: bls12_381::SecretKey = rng.gen();

    let hash = &msg1.0.as_bytes().as_slice();
    let sig1 = key1.sign(hash);
    let sig2 = key2.sign(hash);

    let agg_sig = AggregateSignature::aggregate(vec![&sig1, &sig2]);

    // Matching key and message.
    agg_sig
        .verify_hash(&msg1, [&key1.public(), &key2.public()].into_iter())
        .unwrap();

    // Mismatching message.
    assert!(agg_sig
        .verify_hash(&msg2, [&key1.public(), &key2.public()].into_iter())
        .is_err());

    // Mismatching key.
    assert!(agg_sig
        .verify_hash(
            &msg1,
            [&key1.public(), &key2.public(), &key3.public()].into_iter()
        )
        .is_err());

    assert!(agg_sig
        .verify_hash(&msg1, [&key3.public()].into_iter())
        .is_err());
}

#[test]
fn test_committee_weights_overflow_check() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let attesters: Vec<WeightedAttester> = [u64::MAX / 5; 6]
        .iter()
        .map(|w| WeightedAttester {
            key: rng.gen::<SecretKey>().public(),
            weight: *w,
        })
        .collect();

    // Creation should overflow
    assert_matches!(Committee::new(attesters), Err(_));
}

#[test]
fn test_committee_with_zero_weights() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let attesters: Vec<WeightedAttester> = [1000, 0, 800, 6000, 0, 700]
        .iter()
        .map(|w| WeightedAttester {
            key: rng.gen::<SecretKey>().public(),
            weight: *w,
        })
        .collect();

    // Committee creation should error on zero weight attesters
    assert_matches!(Committee::new(attesters), Err(_));
}
