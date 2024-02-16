use super::*;
use crate::validator::testonly::Setup;
use assert_matches::assert_matches;
use rand::{seq::SliceRandom, Rng};
use std::vec;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::test_encode_random;

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
    test_encode_random::<BlockHeaderHash>(rng);
    test_encode_random::<FinalBlock>(rng);
    test_encode_random::<Signed<ConsensusMsg>>(rng);
    test_encode_random::<PrepareQC>(rng);
    test_encode_random::<CommitQC>(rng);
    test_encode_random::<Msg>(rng);
    test_encode_random::<MsgHash>(rng);
    test_encode_random::<Signers>(rng);
    test_encode_random::<PublicKey>(rng);
    test_encode_random::<Signature>(rng);
    test_encode_random::<AggregateSignature>(rng);
    test_encode_random::<Fork>(rng);
    test_encode_random::<ForkSet>(rng);
    test_encode_random::<Genesis>(rng);
    test_encode_random::<GenesisHash>(rng);
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
        protocol_version: ProtocolVersion::EARLIEST,
        fork: setup.genesis.forks.current().number,
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
    for key in &setup.keys {
        qc.add(&key.sign_msg(qc.message.clone()), &setup.genesis);
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

    let setup1 = Setup::new(rng, 6);
    let setup2 = Setup::new(rng, 6);
    let genesis3 = Genesis {
        validators: ValidatorSet::new(setup1.genesis.validators.iter().take(3).cloned()).unwrap(),
        forks: setup1.genesis.forks.clone(),
    };

    let allow_past_forks = false;
    for i in 0..setup1.keys.len() + 1 {
        let view = rng.gen();
        let mut qc = CommitQC::new(make_replica_commit(rng, view, &setup1), &setup1.genesis);
        for key in &setup1.keys[0..i] {
            qc.add(&key.sign_msg(qc.message.clone()), &setup1.genesis);
        }
        if i >= setup1.genesis.validators.threshold() {
            qc.verify(&setup1.genesis, allow_past_forks).unwrap();
        } else {
            assert_matches!(
                qc.verify(&setup1.genesis, allow_past_forks),
                Err(Error::NotEnoughSigners { .. })
            );
        }

        // Mismatching validator sets.
        assert!(qc.verify(&setup2.genesis, allow_past_forks).is_err());
        assert!(qc.verify(&genesis3, allow_past_forks).is_err());
    }
}

#[test]
fn test_prepare_qc() {
    use PrepareQCVerifyError as Error;
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let setup1 = Setup::new(rng, 6);
    let setup2 = Setup::new(rng, 6);
    let genesis3 = Genesis {
        validators: ValidatorSet::new(setup1.genesis.validators.iter().take(3).cloned()).unwrap(),
        forks: setup1.genesis.forks.clone(),
    };

    let view: ViewNumber = rng.gen();
    let msgs: Vec<_> = (0..3)
        .map(|_| make_replica_prepare(rng, view, &setup1))
        .collect();

    for n in 0..setup1.keys.len() + 1 {
        let mut qc = PrepareQC::new(msgs[0].view.clone());
        for key in &setup1.keys[0..n] {
            qc.add(
                &key.sign_msg(msgs.choose(rng).unwrap().clone()),
                &setup1.genesis,
            );
        }
        if n >= setup1.genesis.validators.threshold() {
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
