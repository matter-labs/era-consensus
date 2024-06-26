use super::*;
use crate::validator::testonly::Setup;
use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{bls12_381, ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::test_encode_random;

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

fn make_batch_msg(rng: &mut impl Rng) -> Batch {
    Batch { number: rng.gen() }
}

#[test]
fn test_batch_qc() {
    use BatchQCVerifyError as Error;
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let setup1 = Setup::new(rng, 6);
    // Completely different genesis.
    let setup2 = Setup::new(rng, 6);
    // Genesis with only a subset of the attesters.
    let genesis3 = {
        let mut genesis3 = (*setup1.genesis).clone();
        genesis3.attesters = Committee::new(
            setup1
                .genesis
                .attesters
                .as_ref()
                .unwrap()
                .iter()
                .take(3)
                .cloned(),
        )
        .unwrap()
        .into();
        genesis3.with_hash()
    };

    let attesters = setup1.genesis.attesters.as_ref().unwrap();

    // Create QCs with increasing number of attesters.
    for i in 0..setup1.attester_keys.len() + 1 {
        let mut qc = BatchQC::new(make_batch_msg(rng)).unwrap();
        for key in &setup1.attester_keys[0..i] {
            qc.add(&key.sign_msg(qc.message.clone()), &setup1.genesis)
                .unwrap();
        }

        let expected_weight: u64 = attesters.iter().take(i).map(|w| w.weight).sum();
        if expected_weight >= attesters.threshold() {
            qc.verify(&setup1.genesis).expect("failed to verify QC");
        } else {
            assert_matches!(
                qc.verify(&setup1.genesis),
                Err(Error::NotEnoughSigners { .. })
            );
        }

        // Mismatching attesters sets.
        assert!(qc.verify(&setup2.genesis).is_err());
        assert!(qc.verify(&genesis3).is_err());
    }
}

#[test]
fn test_attester_committee_weights() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    // Attesters with non-uniform weights
    let setup = Setup::new_with_weights(rng, vec![1000, 600, 800, 6000, 900, 700]);
    // Expected sum of the attesters weights
    let sums = [1000, 1600, 2400, 8400, 9300, 10000];

    let msg = make_batch_msg(rng);
    let mut qc = BatchQC::new(msg.clone()).unwrap();
    for (n, weight) in sums.iter().enumerate() {
        let key = &setup.attester_keys[n];
        qc.add(&key.sign_msg(msg.clone()), &setup.genesis).unwrap();
        assert_eq!(
            setup
                .genesis
                .attesters
                .as_ref()
                .unwrap()
                .weight_of_keys(qc.signatures.keys()),
            *weight
        );
    }
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
