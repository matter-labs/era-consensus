use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::ctx;

use super::*;
use crate::validator::{messages::tests::genesis_v2, testonly::Setup, ChainId, Signed, ViewNumber};

#[test]
fn test_replica_timeout_verify() {
    let genesis = genesis_v2();
    let timeout = replica_timeout();
    assert!(timeout.verify(&genesis).is_ok());

    // Wrong view
    let mut wrong_raw_genesis = genesis.0.clone();
    wrong_raw_genesis.chain_id = ChainId(1);
    let wrong_genesis = wrong_raw_genesis.with_hash();
    assert_matches!(
        timeout.verify(&wrong_genesis),
        Err(ReplicaTimeoutVerifyError::BadView(_))
    );

    // Invalid high vote
    let mut timeout = replica_timeout();
    timeout.high_vote.as_mut().unwrap().view.genesis = wrong_genesis.hash();
    assert_matches!(
        timeout.verify(&genesis),
        Err(ReplicaTimeoutVerifyError::InvalidHighVote(_))
    );

    // Invalid high QC
    let mut timeout = replica_timeout();
    timeout.high_qc.as_mut().unwrap().message.view.genesis = wrong_genesis.hash();
    assert_matches!(
        timeout.verify(&genesis),
        Err(ReplicaTimeoutVerifyError::InvalidHighQC(_))
    );
}

#[test]
fn test_timeout_qc_high_vote() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    // This will create equally weighted validators
    let setup = Setup::new(rng, 6);

    let view_num: ViewNumber = rng.gen();
    let msg_a = setup.make_replica_timeout_v2(rng, view_num);
    let msg_b = setup.make_replica_timeout_v2(rng, view_num);
    let msg_c = setup.make_replica_timeout_v2(rng, view_num);

    // Case with 1 subquorum.
    let mut qc = TimeoutQC::new(msg_a.view);

    for key in &setup.validator_keys {
        qc.add(&key.sign_msg(msg_a.clone()), &setup.genesis)
            .unwrap();
    }

    assert!(qc.high_vote(&setup.genesis).is_some());

    // Case with 2 subquorums.
    let mut qc = TimeoutQC::new(msg_a.view);

    for key in &setup.validator_keys[0..3] {
        qc.add(&key.sign_msg(msg_a.clone()), &setup.genesis)
            .unwrap();
    }

    for key in &setup.validator_keys[3..6] {
        qc.add(&key.sign_msg(msg_b.clone()), &setup.genesis)
            .unwrap();
    }

    assert!(qc.high_vote(&setup.genesis).is_none());

    // Case with no subquorums.
    let mut qc = TimeoutQC::new(msg_a.view);

    for key in &setup.validator_keys[0..2] {
        qc.add(&key.sign_msg(msg_a.clone()), &setup.genesis)
            .unwrap();
    }

    for key in &setup.validator_keys[2..4] {
        qc.add(&key.sign_msg(msg_b.clone()), &setup.genesis)
            .unwrap();
    }

    for key in &setup.validator_keys[4..6] {
        qc.add(&key.sign_msg(msg_c.clone()), &setup.genesis)
            .unwrap();
    }

    assert!(qc.high_vote(&setup.genesis).is_none());
}

#[test]
fn test_timeout_qc_high_qc() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 3);
    let view = View {
        genesis: setup.genesis.hash(),
        number: ViewNumber(100),
        epoch: EpochNumber(0),
    };
    let mut qc = TimeoutQC::new(view);

    // No high QC
    assert!(qc.high_qc().is_none());

    // Add signatures with different high QC views
    for i in 0..3 {
        let high_vote_view = view.number;
        let high_qc_view = ViewNumber(view.number.0 - i as u64);
        let msg = ReplicaTimeout {
            view: setup.make_view_v2(view.number),
            high_vote: Some(setup.make_replica_commit_v2(rng, high_vote_view)),
            high_qc: Some(setup.make_commit_qc_v2(rng, high_qc_view)),
        };
        qc.add(
            &setup.validator_keys[i].sign_msg(msg.clone()),
            &setup.genesis,
        )
        .unwrap();
    }

    assert_eq!(qc.high_qc().unwrap().message.view.number, view.number);
}

#[test]
fn test_timeout_qc_add() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 3);
    let view = rng.gen();
    let msg = setup.make_replica_timeout_v2(rng, view);
    let mut qc = TimeoutQC::new(msg.view);

    // Add the first signature
    assert!(qc.map.is_empty());
    assert!(qc
        .add(
            &setup.validator_keys[0].sign_msg(msg.clone()),
            &setup.genesis
        )
        .is_ok());
    assert_eq!(qc.map.len(), 1);
    assert_eq!(qc.map.values().next().unwrap().count(), 1);

    // Try to add a message from a signer not in committee
    assert_matches!(
        qc.add(
            &rng.gen::<validator::SecretKey>().sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(TimeoutQCAddError::SignerNotInCommittee { .. })
    );

    // Try to add the same message already added by same validator
    assert_matches!(
        qc.add(
            &setup.validator_keys[0].sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(TimeoutQCAddError::DuplicateSigner { .. })
    );

    // Try to add an invalid signature
    assert_matches!(
        qc.add(
            &Signed {
                msg: msg.clone(),
                key: setup.validator_keys[1].public(),
                sig: rng.gen()
            },
            &setup.genesis
        ),
        Err(TimeoutQCAddError::BadSignature(_))
    );

    // Try to add a message with a different view
    let mut msg1 = msg.clone();
    msg1.view.number = view.next();
    assert_matches!(
        qc.add(&setup.validator_keys[1].sign_msg(msg1), &setup.genesis),
        Err(TimeoutQCAddError::InconsistentViews)
    );

    // Try to add an invalid message
    let mut wrong_genesis = setup.genesis.clone().0;
    wrong_genesis.chain_id = rng.gen();
    assert_matches!(
        qc.add(
            &setup.validator_keys[1].sign_msg(msg.clone()),
            &wrong_genesis.with_hash()
        ),
        Err(TimeoutQCAddError::InvalidMessage(_))
    );

    // Add same message signed by another validator.
    assert!(qc
        .add(
            &setup.validator_keys[1].sign_msg(msg.clone()),
            &setup.genesis
        )
        .is_ok());
    assert_eq!(qc.map.len(), 1);
    assert_eq!(qc.map.values().next().unwrap().count(), 2);

    // Add a different message signed by another validator.
    let msg2 = setup.make_replica_timeout_v2(rng, view);
    assert!(qc
        .add(
            &setup.validator_keys[2].sign_msg(msg2.clone()),
            &setup.genesis
        )
        .is_ok());
    assert_eq!(qc.map.len(), 2);
    // The order of the messages is not guaranteed to be chronological.
    assert_eq!(qc.map.values().next().unwrap().count(), 1);
    assert_eq!(qc.map.values().last().unwrap().count(), 2);
}

#[test]
fn test_timeout_qc_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new(rng, 6);
    setup.push_blocks_v2(rng, 2);
    let view = rng.gen();
    let qc = setup.make_timeout_qc_v2(rng, view, None);

    // Verify the QC
    assert!(qc.verify(&setup.genesis).is_ok());

    // QC with bad view
    let mut qc1 = qc.clone();
    qc1.view = rng.gen();
    assert_matches!(
        qc1.verify(&setup.genesis),
        Err(TimeoutQCVerifyError::BadView(_))
    );

    // QC with message with inconsistent view
    let mut qc2 = qc.clone();
    qc2.map.insert(
        ReplicaTimeout {
            view: qc2.view.next_view(),
            high_vote: None,
            high_qc: None,
        },
        Signers::new(setup.genesis.validators.len()),
    );
    assert_matches!(
        qc2.verify(&setup.genesis),
        Err(TimeoutQCVerifyError::InconsistentView(_))
    );

    // QC with message with wrong signer length
    let mut qc3 = qc.clone();
    qc3.map.insert(
        ReplicaTimeout {
            view: qc3.view,
            high_vote: None,
            high_qc: None,
        },
        Signers::new(setup.genesis.validators.len() + 1),
    );
    assert_matches!(
        qc3.verify(&setup.genesis),
        Err(TimeoutQCVerifyError::WrongSignersLength(_))
    );

    // QC with message with no signers
    let mut qc4 = qc.clone();
    qc4.map.insert(
        ReplicaTimeout {
            view: qc4.view,
            high_vote: None,
            high_qc: None,
        },
        Signers::new(setup.genesis.validators.len()),
    );
    assert_matches!(
        qc4.verify(&setup.genesis),
        Err(TimeoutQCVerifyError::NoSignersAssigned(_))
    );

    // QC with overlapping signers
    let mut qc5 = qc.clone();
    let mut signers = Signers::new(setup.genesis.validators.len());
    signers
        .0
        .set(rng.gen_range(0..setup.genesis.validators.len()), true);
    qc5.map.insert(
        ReplicaTimeout {
            view: qc5.view,
            high_vote: None,
            high_qc: None,
        },
        signers,
    );
    assert_matches!(
        qc5.verify(&setup.genesis),
        Err(TimeoutQCVerifyError::OverlappingSignatureSet(_))
    );

    // QC with invalid message
    let mut qc6 = qc.clone();
    let (mut timeout, signers) = qc6.map.pop_first().unwrap();
    timeout.high_qc = Some(rng.gen());
    qc6.map.insert(timeout, signers);
    assert_matches!(
        qc6.verify(&setup.genesis),
        Err(TimeoutQCVerifyError::InvalidMessage(_, _))
    );

    // QC with not enough weight
    let mut qc7 = qc.clone();
    let (timeout, mut signers) = qc7.map.pop_first().unwrap();
    signers.0.set(0, false);
    signers.0.set(4, false);
    qc7.map.insert(timeout, signers);
    assert_matches!(
        qc7.verify(&setup.genesis),
        Err(TimeoutQCVerifyError::NotEnoughWeight { .. })
    );

    // QC with bad signature
    let mut qc8 = qc.clone();
    qc8.signature = rng.gen();
    assert_matches!(
        qc8.verify(&setup.genesis),
        Err(TimeoutQCVerifyError::BadSignature(_))
    );
}
