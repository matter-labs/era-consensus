use crate::validator::{testonly::Setup, Genesis, ValidatorSet};

use super::*;
use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::{test_encode, test_encode_random};

#[test]
fn test_byte_encoding() {
    let key = SecretKey::generate();
    assert_eq!(
        key.public(),
        <SecretKey as ByteFmt>::decode(&ByteFmt::encode(&key))
            .unwrap()
            .public()
    );
    assert_eq!(
        key.public(),
        ByteFmt::decode(&ByteFmt::encode(&key.public())).unwrap()
    );
}

#[test]
fn test_text_encoding() {
    let key = SecretKey::generate();
    let t1 = TextFmt::encode(&key);
    let t2 = TextFmt::encode(&key.public());
    assert_eq!(
        key.public(),
        Text::new(&t1).decode::<SecretKey>().unwrap().public()
    );
    assert_eq!(key.public(), Text::new(&t2).decode().unwrap());
    assert!(Text::new(&t1).decode::<PublicKey>().is_err());
    assert!(Text::new(&t2).decode::<SecretKey>().is_err());
}

#[test]
fn test_schema_encoding() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<SignedBatchMsg<L1Batch>>(rng);
    let key = rng.gen::<SecretKey>().public();
    test_encode(rng, &key);
    test_encode_random::<Signature>(rng);
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
fn test_l1_batch_qc() {
    use L1BatchQCVerifyError as Error;
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let setup1 = Setup::new(rng, 6);
    let setup2 = Setup::new(rng, 6);
    let genesis3 = Genesis {
        validators: ValidatorSet::new(setup1.genesis.validators.iter().take(3).cloned()).unwrap(),
        attesters: AttesterSet::new(setup1.genesis.attesters.iter().take(3).cloned()).unwrap(),
        fork: setup1.genesis.fork.clone(),
    };

    for i in 0..setup1.attester_keys.len() + 1 {
        let mut qc = L1BatchQC::new(L1Batch::default(), &setup1.genesis);
        for key in &setup1.attester_keys[0..i] {
            qc.add(&key.sign_batch_msg(qc.message.clone()), &setup1.genesis);
        }
        if i >= setup1.genesis.attesters.threshold() {
            assert!(qc.verify(&setup1.genesis).is_ok());
        } else {
            assert_matches!(
                qc.verify(&setup1.genesis),
                Err(Error::NotEnoughSigners { .. })
            );
        }

        // Mismatching validator sets.
        assert!(qc.verify(&setup2.genesis).is_err());
        assert!(qc.verify(&genesis3).is_err());
    }
}
