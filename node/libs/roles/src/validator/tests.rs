use super::*;
use crate::validator::testonly::Setup;
use assert_matches::assert_matches;
use rand::{seq::SliceRandom, Rng};
use std::vec;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::{testonly::test_encode_random, ProtoFmt};

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
    assert_eq!(sk, Text::new(&t).decode::<SecretKey>().unwrap());

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

    let msg_hash: MsgHash = rng.gen();
    let t = TextFmt::encode(&msg_hash);
    assert_eq!(msg_hash, Text::new(&t).decode::<MsgHash>().unwrap());

    let genesis_hash: GenesisHash = rng.gen();
    let t = TextFmt::encode(&genesis_hash);
    assert_eq!(genesis_hash, Text::new(&t).decode::<GenesisHash>().unwrap());
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<PayloadHash>(rng);
    test_encode_random::<BlockHeader>(rng);
    test_encode_random::<FinalBlock>(rng);
    test_encode_random::<Signed<ConsensusMsg>>(rng);
    test_encode_random::<PrepareQC>(rng);
    test_encode_random::<CommitQC>(rng);
    test_encode_random::<Msg>(rng);
    test_encode_random::<MsgHash>(rng);
    test_encode_random::<Signers>(rng);
    test_encode_random::<PublicKey>(rng);
    test_encode_random::<Signature>(rng);
    test_encode_random::<Genesis>(rng);
    test_encode_random::<AggregateSignature>(rng);
    test_encode_random::<GenesisHash>(rng);
    test_encode_random::<LeaderSelectionMode>(rng);
}

#[test]
fn test_genesis_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let genesis = Setup::new(rng, 1).genesis.clone();
    assert!(genesis.verify().is_ok());
    assert!(Genesis::read(&genesis.build()).is_ok());

    let mut genesis = (*genesis).clone();
    genesis.leader_selection = LeaderSelectionMode::Sticky(rng.gen());
    let genesis = genesis.with_hash();
    assert!(genesis.verify().is_err());
    assert!(Genesis::read(&genesis.build()).is_err())
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

fn make_view(number: ViewNumber, setup: &Setup) -> View {
    View {
        genesis: setup.genesis.hash(),
        number,
    }
}

fn make_replica_commit(rng: &mut impl Rng, view: ViewNumber, setup: &Setup) -> ReplicaCommit {
    ReplicaCommit {
        view: make_view(view, setup),
        proposal: rng.gen(),
    }
}

fn make_commit_qc(rng: &mut impl Rng, view: ViewNumber, setup: &Setup) -> CommitQC {
    let mut qc = CommitQC::new(make_replica_commit(rng, view, setup), &setup.genesis);
    for key in &setup.validator_keys {
        qc.add(&key.sign_msg(qc.message.clone()), &setup.genesis)
            .unwrap();
    }
    qc
}

fn make_replica_prepare(rng: &mut impl Rng, view: ViewNumber, setup: &Setup) -> ReplicaPrepare {
    ReplicaPrepare {
        view: make_view(view, setup),
        high_vote: {
            let view = ViewNumber(rng.gen_range(0..view.0));
            Some(make_replica_commit(rng, view, setup))
        },
        high_qc: {
            let view = ViewNumber(rng.gen_range(0..view.0));
            Some(make_commit_qc(rng, view, setup))
        },
    }
}

#[test]
fn test_commit_qc() {
    use CommitQCVerifyError as Error;
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    // This will create equally weighted validators
    let setup1 = Setup::new(rng, 6);
    let setup2 = Setup::new(rng, 6);
    let mut genesis3 = (*setup1.genesis).clone();
    genesis3.validators =
        Committee::new(setup1.genesis.validators.iter().take(3).cloned()).unwrap();
    let genesis3 = genesis3.with_hash();

    for i in 0..setup1.validator_keys.len() + 1 {
        let view = rng.gen();
        let mut qc = CommitQC::new(make_replica_commit(rng, view, &setup1), &setup1.genesis);
        for key in &setup1.validator_keys[0..i] {
            qc.add(&key.sign_msg(qc.message.clone()), &setup1.genesis)
                .unwrap();
        }
        let expected_weight: u64 = setup1
            .genesis
            .attesters
            .as_ref()
            .unwrap()
            .iter()
            .take(i)
            .map(|w| w.weight)
            .sum();
        if expected_weight >= setup1.genesis.validators.threshold() {
            qc.verify(&setup1.genesis).unwrap();
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

#[test]
fn test_commit_qc_add_errors() {
    use CommitQCAddError as Error;
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 2);
    let view = rng.gen();
    let mut qc = CommitQC::new(make_replica_commit(rng, view, &setup), &setup.genesis);
    let msg = qc.message.clone();
    // Add the message
    assert_matches!(
        qc.add(
            &setup.validator_keys[0].sign_msg(msg.clone()),
            &setup.genesis
        ),
        Ok(())
    );

    // Try to add a message for a different view
    let mut msg1 = msg.clone();
    msg1.view.number = view.next();
    assert_matches!(
        qc.add(&setup.validator_keys[0].sign_msg(msg1), &setup.genesis),
        Err(Error::InconsistentMessages { .. })
    );

    // Try to add a message from a signer not in committee
    assert_matches!(
        qc.add(
            &rng.gen::<SecretKey>().sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(Error::SignerNotInCommittee { .. })
    );

    // Try to add the same message already added by same validator
    assert_matches!(
        qc.add(
            &setup.validator_keys[0].sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(Error::Exists { .. })
    );

    // Add same message signed by another validator.
    assert_matches!(
        qc.add(&setup.validator_keys[1].sign_msg(msg), &setup.genesis),
        Ok(())
    );
}

#[test]
fn test_prepare_qc() {
    use PrepareQCVerifyError as Error;
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    // This will create equally weighted validators
    let setup1 = Setup::new(rng, 6);
    let setup2 = Setup::new(rng, 6);
    let mut genesis3 = (*setup1.genesis).clone();
    genesis3.validators =
        Committee::new(setup1.genesis.validators.iter().take(3).cloned()).unwrap();
    let genesis3 = genesis3.with_hash();

    let view: ViewNumber = rng.gen();
    let msgs: Vec<_> = (0..3)
        .map(|_| make_replica_prepare(rng, view, &setup1))
        .collect();

    for n in 0..setup1.validator_keys.len() + 1 {
        let mut qc = PrepareQC::new(msgs[0].view.clone());
        for key in &setup1.validator_keys[0..n] {
            qc.add(
                &key.sign_msg(msgs.choose(rng).unwrap().clone()),
                &setup1.genesis,
            )
            .unwrap();
        }
        let expected_weight: u64 = setup1
            .genesis
            .attesters
            .as_ref()
            .unwrap()
            .iter()
            .take(n)
            .map(|w| w.weight)
            .sum();
        if expected_weight >= setup1.genesis.validators.threshold() {
            qc.verify(&setup1.genesis).unwrap();
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

#[test]
fn test_prepare_qc_add_errors() {
    use PrepareQCAddError as Error;
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 2);
    let view = rng.gen();
    let msg = make_replica_prepare(rng, view, &setup);
    let mut qc = PrepareQC::new(msg.view.clone());
    let msg = make_replica_prepare(rng, view, &setup);

    // Add the message
    assert_matches!(
        qc.add(
            &setup.validator_keys[0].sign_msg(msg.clone()),
            &setup.genesis
        ),
        Ok(())
    );

    // Try to add a message for a different view
    let mut msg1 = msg.clone();
    msg1.view.number = view.next();
    assert_matches!(
        qc.add(&setup.validator_keys[0].sign_msg(msg1), &setup.genesis),
        Err(Error::InconsistentViews { .. })
    );

    // Try to add a message from a signer not in committee
    assert_matches!(
        qc.add(
            &rng.gen::<SecretKey>().sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(Error::SignerNotInCommittee { .. })
    );

    // Try to add the same message already added by same validator
    assert_matches!(
        qc.add(
            &setup.validator_keys[0].sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(Error::Exists { .. })
    );

    // Try to add a message for a validator that already added another message
    let msg2 = make_replica_prepare(rng, view, &setup);
    assert_matches!(
        qc.add(&setup.validator_keys[0].sign_msg(msg2), &setup.genesis),
        Err(Error::Exists { .. })
    );

    // Add same message signed by another validator.
    assert_matches!(
        qc.add(
            &setup.validator_keys[1].sign_msg(msg.clone()),
            &setup.genesis
        ),
        Ok(())
    );
}

#[test]
fn test_validator_committee_weights() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    // Validators with non-uniform weights
    let setup = Setup::new_with_weights(rng, vec![1000, 600, 800, 6000, 900, 700]);
    // Expected sum of the validators weights
    let sums = [1000, 1600, 2400, 8400, 9300, 10000];

    let view: ViewNumber = rng.gen();
    let msg = make_replica_prepare(rng, view, &setup);
    let mut qc = PrepareQC::new(msg.view.clone());
    for (n, weight) in sums.iter().enumerate() {
        let key = &setup.validator_keys[n];
        qc.add(&key.sign_msg(msg.clone()), &setup.genesis).unwrap();
        let signers = &qc.map[&msg];
        assert_eq!(setup.genesis.validators.weight(signers), *weight);
    }
}

#[test]
fn test_committee_weights_overflow_check() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let validators: Vec<WeightedValidator> = [u64::MAX / 5; 6]
        .iter()
        .map(|w| WeightedValidator {
            key: rng.gen::<SecretKey>().public(),
            weight: *w,
        })
        .collect();

    // Creation should overflow
    assert_matches!(Committee::new(validators), Err(_));
}

#[test]
fn test_committee_with_zero_weights() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let validators: Vec<WeightedValidator> = [1000, 0, 800, 6000, 0, 700]
        .iter()
        .map(|w| WeightedValidator {
            key: rng.gen::<SecretKey>().public(),
            weight: *w,
        })
        .collect();

    // Committee creation should error on zero weight validators
    assert_matches!(Committee::new(validators), Err(_));
}
