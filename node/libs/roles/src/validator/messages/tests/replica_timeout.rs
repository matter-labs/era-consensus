use super::*;
use assert_matches::assert_matches;
use rand::seq::SliceRandom as _;
use zksync_concurrency::ctx;

#[test]
fn test_timeout_qc() {
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
        .map(|_| setup1.make_replica_timeout(rng, view))
        .collect();

    for n in 0..setup1.validator_keys.len() + 1 {
        let mut qc = TimeoutQC::new(msgs[0].view.clone());
        for key in &setup1.validator_keys[0..n] {
            qc.add(
                &key.sign_msg(msgs.choose(rng).unwrap().clone()),
                &setup1.genesis,
            )
            .unwrap();
        }
        let expected_weight: u64 = setup1
            .genesis
            .validators
            .iter()
            .take(n)
            .map(|w| w.weight)
            .sum();
        if expected_weight >= setup1.genesis.validators.quorum_threshold() {
            qc.verify(&setup1.genesis).unwrap();
        } else {
            assert_matches!(
                qc.verify(&setup1.genesis),
                Err(TimeoutQCVerifyError::NotEnoughSigners { .. })
            );
        }

        // Mismatching validator sets.
        assert!(qc.verify(&setup2.genesis).is_err());
        assert!(qc.verify(&genesis3).is_err());
    }
}

#[test]
fn test_timeout_qc_high_vote() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    // This will create equally weighted validators
    let setup = Setup::new(rng, 6);

    let view_num: ViewNumber = rng.gen();
    let msg_a = setup.make_replica_timeout(rng, view_num);
    let msg_b = setup.make_replica_timeout(rng, view_num);
    let msg_c = setup.make_replica_timeout(rng, view_num);

    // Case with 1 subquorum.
    let mut qc = TimeoutQC::new(msg_a.view.clone());

    for key in &setup.validator_keys {
        qc.add(&key.sign_msg(msg_a.clone()), &setup.genesis)
            .unwrap();
    }

    assert!(qc.high_vote(&setup.genesis).is_some());

    // Case with 2 subquorums.
    let mut qc = TimeoutQC::new(msg_a.view.clone());

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
    let mut qc = TimeoutQC::new(msg_a.view.clone());

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
fn test_timeout_qc_add_errors() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 2);
    let view = rng.gen();
    let msg = setup.make_replica_timeout(rng, view);
    let mut qc = TimeoutQC::new(msg.view.clone());
    let msg = setup.make_replica_timeout(rng, view);

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
        Err(TimeoutQCAddError::InconsistentViews { .. })
    );

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

    // Try to add a message for a validator that already added another message
    let msg2 = setup.make_replica_timeout(rng, view);
    assert_matches!(
        qc.add(&setup.validator_keys[0].sign_msg(msg2), &setup.genesis),
        Err(TimeoutQCAddError::DuplicateSigner { .. })
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
