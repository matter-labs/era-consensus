use super::*;
use ::schema::testonly::test_encode_random;
use concurrency::ctx;
use crypto::{ByteFmt, Text, TextFmt};
use rand::Rng;
use std::vec;

#[test]
fn test_byte_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let sk: SecretKey = rng.gen();
    assert_eq!(
        sk.public(),
        <SecretKey as ByteFmt>::decode(&ByteFmt::encode(&sk))
            .unwrap()
            .public()
    );

    let pk: PublicKey = rng.gen();
    assert_eq!(pk, ByteFmt::decode(&ByteFmt::encode(&pk)).unwrap());

    let sig: Signature = rng.gen();
    assert_eq!(sig, ByteFmt::decode(&ByteFmt::encode(&sig)).unwrap());

    let agg_sig: AggregateSignature = rng.gen();
    assert_eq!(
        agg_sig,
        ByteFmt::decode(&ByteFmt::encode(&agg_sig)).unwrap()
    );

    let final_block: FinalBlock = rng.gen();
    assert_eq!(
        final_block,
        ByteFmt::decode(&ByteFmt::encode(&final_block)).unwrap()
    );

    let msg_hash: MsgHash = rng.gen();
    assert_eq!(
        msg_hash,
        ByteFmt::decode(&ByteFmt::encode(&msg_hash)).unwrap()
    );
}

#[test]
fn test_text_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let sk: SecretKey = rng.gen();
    let t = TextFmt::encode(&sk);
    assert_eq!(
        sk.public(),
        Text::new(&t).decode::<SecretKey>().unwrap().public()
    );

    let pk: PublicKey = rng.gen();
    let t = TextFmt::encode(&pk);
    assert_eq!(pk, Text::new(&t).decode::<PublicKey>().unwrap());

    let sig: Signature = rng.gen();
    let t = TextFmt::encode(&sig);
    assert_eq!(sig, Text::new(&t).decode::<Signature>().unwrap());

    let agg_sig: AggregateSignature = rng.gen();
    let t = TextFmt::encode(&agg_sig);
    assert_eq!(
        agg_sig,
        Text::new(&t).decode::<AggregateSignature>().unwrap()
    );

    let block_header_hash: BlockHeaderHash = rng.gen();
    let t = TextFmt::encode(&block_header_hash);
    assert_eq!(
        block_header_hash,
        Text::new(&t).decode::<BlockHeaderHash>().unwrap()
    );

    let final_block: FinalBlock = rng.gen();
    let t = TextFmt::encode(&final_block);
    assert_eq!(final_block, Text::new(&t).decode::<FinalBlock>().unwrap());

    let msg_hash: MsgHash = rng.gen();
    let t = TextFmt::encode(&msg_hash);
    assert_eq!(msg_hash, Text::new(&t).decode::<MsgHash>().unwrap());
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<_, PayloadHash>(rng);
    test_encode_random::<_, BlockHeader>(rng);
    test_encode_random::<_, BlockHeaderHash>(rng);
    test_encode_random::<_, FinalBlock>(rng);
    test_encode_random::<_, Signed<ConsensusMsg>>(rng);
    test_encode_random::<_, PrepareQC>(rng);
    test_encode_random::<_, CommitQC>(rng);
    test_encode_random::<_, Msg>(rng);
    test_encode_random::<_, MsgHash>(rng);
    test_encode_random::<_, Signers>(rng);
    test_encode_random::<_, PublicKey>(rng);
    test_encode_random::<_, Signature>(rng);
    test_encode_random::<_, AggregateSignature>(rng);
}

#[test]
fn test_signature_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let msg1: MsgHash = rng.gen();
    let msg2: MsgHash = rng.gen();

    let key1 = SecretKey::generate(rng.gen());
    let key2 = SecretKey::generate(rng.gen());

    let sig1 = key1.sign_hash(&msg1);

    // Matching key and message.
    assert!(sig1.verify_hash(&msg1, &key1.public()).is_ok());

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

    let key1 = SecretKey::generate(rng.gen());
    let key2 = SecretKey::generate(rng.gen());

    let sig1 = key1.sign_hash(&msg1);
    let sig2 = key2.sign_hash(&msg2);

    let agg_sig = AggregateSignature::aggregate(vec![&sig1, &sig2]).unwrap();

    // Matching key and message.
    assert!(agg_sig
        .verify_hash([(msg1, &key1.public()), (msg2, &key2.public())].into_iter())
        .is_ok());

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
fn test_commit_qc() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let sk1: SecretKey = rng.gen();
    let sk2: SecretKey = rng.gen();
    let sk3: SecretKey = rng.gen();

    let msg: ReplicaCommit = rng.gen();

    let validator_set1 = ValidatorSet::new(vec![
        sk1.public(),
        sk2.public(),
        sk3.public(),
        rng.gen(),
        rng.gen(),
    ])
    .unwrap();
    let validator_set2 =
        ValidatorSet::new(vec![rng.gen(), rng.gen(), rng.gen(), rng.gen(), rng.gen()]).unwrap();
    //let validator_set3 = ValidatorSet::new(vec![sk1.public(), sk2.public(), sk3.public()]).unwrap();

    let qc = CommitQC::from(
        &[sk1.sign_msg(msg), sk2.sign_msg(msg), sk3.sign_msg(msg)],
        &validator_set1,
    )
    .unwrap();

    // Matching validator set and enough signers.
    assert!(qc.verify(&validator_set1, 1).is_ok());
    assert!(qc.verify(&validator_set1, 2).is_ok());
    assert!(qc.verify(&validator_set1, 3).is_ok());

    // Not enough signers.
    assert!(qc.verify(&validator_set1, 4).is_err());

    // Mismatching validator sets.
    assert!(qc.verify(&validator_set2, 3).is_err());
    //assert!(qc.verify(&validator_set3, 3).is_err());
}

#[test]
fn test_prepare_qc() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let sk1: SecretKey = rng.gen();
    let sk2: SecretKey = rng.gen();
    let sk3: SecretKey = rng.gen();

    let view: ViewNumber = rng.gen();
    let mut msg1: ReplicaPrepare = rng.gen();
    let mut msg2: ReplicaPrepare = rng.gen();
    msg1.view = view;
    msg2.view = view;

    let validator_set1 = ValidatorSet::new(vec![
        sk1.public(),
        sk2.public(),
        sk3.public(),
        rng.gen(),
        rng.gen(),
    ])
    .unwrap();
    let validator_set2 =
        ValidatorSet::new(vec![rng.gen(), rng.gen(), rng.gen(), rng.gen(), rng.gen()]).unwrap();
    let validator_set3 = ValidatorSet::new(vec![sk1.public(), sk2.public(), sk3.public()]).unwrap();

    let agg_qc = PrepareQC::from(
        &[
            sk1.sign_msg(msg1.clone()),
            sk2.sign_msg(msg2),
            sk3.sign_msg(msg1),
        ],
        &validator_set1,
    )
    .unwrap();

    // Matching validator set and enough signers.
    assert!(agg_qc.verify(view, &validator_set1, 1).is_ok());
    assert!(agg_qc.verify(view, &validator_set1, 2).is_ok());
    assert!(agg_qc.verify(view, &validator_set1, 3).is_ok());

    // Not enough signers.
    assert!(agg_qc.verify(view, &validator_set1, 4).is_err());

    // Mismatching validator sets.
    assert!(agg_qc.verify(view, &validator_set2, 3).is_err());
    assert!(agg_qc.verify(view, &validator_set3, 3).is_err());
}
