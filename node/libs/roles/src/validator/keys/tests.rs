use std::vec;

use rand::Rng as _;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::test_encode_random;

use super::*;
use crate::validator::MsgHash;

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

    let key1: SecretKey = rng.gen();
    let key2: SecretKey = rng.gen();

    let sig1 = key1.sign_hash(&msg1);
    let sig2 = key2.sign_hash(&msg2);

    let agg_sig = AggregateSignature::aggregate(vec![&sig1, &sig2]);

    // Matching key and message.
    agg_sig
        .verify_hash([(msg1, &key1.public()), (msg2, &key2.public())].into_iter())
        .unwrap();

    // Mismatching message.
    assert!(agg_sig
        .verify_hash([(msg2, &key1.public()), (msg1, &key2.public())].into_iter())
        .is_err());

    // Mismatching key.
    assert!(agg_sig
        .verify_hash([(msg1, &key2.public()), (msg2, &key1.public())].into_iter())
        .is_err());
}

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

    let pop: ProofOfPossession = rng.gen();
    assert_eq!(pop, ByteFmt::decode(&ByteFmt::encode(&pop)).unwrap());
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

    let pop: ProofOfPossession = rng.gen();
    let t = TextFmt::encode(&pop);
    assert_eq!(pop, Text::new(&t).decode::<ProofOfPossession>().unwrap());

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

    test_encode_random::<PublicKey>(rng);
    test_encode_random::<Signature>(rng);
    test_encode_random::<AggregateSignature>(rng);
}
