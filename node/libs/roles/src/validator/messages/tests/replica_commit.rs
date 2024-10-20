use super::*;
use assert_matches::assert_matches;
use zksync_concurrency::ctx;

#[test]
fn test_commit_qc() {
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
        let mut qc = CommitQC::new(setup1.make_replica_commit(rng, view), &setup1.genesis);
        for key in &setup1.validator_keys[0..i] {
            qc.add(&key.sign_msg(qc.message.clone()), &setup1.genesis)
                .unwrap();
        }
        let expected_weight: u64 = setup1
            .genesis
            .validators
            .iter()
            .take(i)
            .map(|w| w.weight)
            .sum();
        if expected_weight >= setup1.genesis.validators.quorum_threshold() {
            qc.verify(&setup1.genesis).unwrap();
        } else {
            assert_matches!(
                qc.verify(&setup1.genesis),
                Err(CommitQCVerifyError::NotEnoughSigners { .. })
            );
        }

        // Mismatching validator sets.
        assert!(qc.verify(&setup2.genesis).is_err());
        assert!(qc.verify(&genesis3).is_err());
    }
}

#[test]
fn test_commit_qc_add_errors() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 2);
    let view = rng.gen();
    let mut qc = CommitQC::new(setup.make_replica_commit(rng, view), &setup.genesis);
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
        Err(CommitQCAddError::InconsistentMessages { .. })
    );

    // Try to add a message from a signer not in committee
    assert_matches!(
        qc.add(
            &rng.gen::<validator::SecretKey>().sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(CommitQCAddError::SignerNotInCommittee { .. })
    );

    // Try to add the same message already added by same validator
    assert_matches!(
        qc.add(
            &setup.validator_keys[0].sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(CommitQCAddError::DuplicateSigner { .. })
    );

    // Add same message signed by another validator.
    assert_matches!(
        qc.add(&setup.validator_keys[1].sign_msg(msg), &setup.genesis),
        Ok(())
    );
}
