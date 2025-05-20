use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::ctx;

use super::*;
use crate::validator::{messages::tests::genesis_v1, testonly::Setup, ChainId, Signed};

#[test]
fn test_replica_commit_verify() {
    let mut genesis = genesis_v1();
    let commit = replica_commit();
    assert!(commit.verify(&genesis).is_ok());

    // Wrong view
    genesis.0.chain_id = ChainId(1);
    let wrong_genesis = genesis.0.with_hash();
    assert_matches!(
        commit.verify(&wrong_genesis),
        Err(ReplicaCommitVerifyError::BadView(_))
    );
}

#[test]
fn test_commit_qc_add() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 2);
    let view = rng.gen();
    let mut qc = CommitQC::new(setup.make_replica_commit_v1(rng, view), &setup.genesis);
    let msg = qc.message.clone();

    // Add the first signature
    assert_eq!(qc.signers.count(), 0);
    assert!(qc
        .add(
            &setup.validator_keys[0].sign_msg(msg.clone()),
            &setup.genesis
        )
        .is_ok());
    assert_eq!(qc.signers.count(), 1);

    // Try to add a signature from a signer not in committee
    assert_matches!(
        qc.add(
            &rng.gen::<validator::SecretKey>().sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(CommitQCAddError::SignerNotInCommittee { .. })
    );

    // Try to add a signature from the same validator
    assert_matches!(
        qc.add(
            &setup.validator_keys[0].sign_msg(msg.clone()),
            &setup.genesis
        ),
        Err(CommitQCAddError::DuplicateSigner { .. })
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
        Err(CommitQCAddError::BadSignature(_))
    );

    // Try to add a signature for a different message
    let mut msg1 = msg.clone();
    msg1.view.number = view.next();
    assert_matches!(
        qc.add(&setup.validator_keys[1].sign_msg(msg1), &setup.genesis),
        Err(CommitQCAddError::InconsistentMessages)
    );

    // Try to add an invalid message
    let mut wrong_genesis = setup.genesis.clone().0;
    wrong_genesis.chain_id = rng.gen();
    assert_matches!(
        qc.add(
            &setup.validator_keys[1].sign_msg(msg.clone()),
            &wrong_genesis.with_hash()
        ),
        Err(CommitQCAddError::InvalidMessage(_))
    );

    // Add same message signed by another validator.
    assert_matches!(
        qc.add(&setup.validator_keys[1].sign_msg(msg), &setup.genesis),
        Ok(())
    );
    assert_eq!(qc.signers.count(), 2);
}

#[test]
fn test_commit_qc_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 6);
    let view = rng.gen();
    let qc = setup.make_commit_qc_v1(rng, view);

    // Verify the QC
    assert!(qc.verify(&setup.genesis).is_ok());

    // QC with bad message
    let mut qc1 = qc.clone();
    qc1.message.view.genesis = rng.gen();
    assert_matches!(
        qc1.verify(&setup.genesis),
        Err(CommitQCVerifyError::InvalidMessage(_))
    );

    // QC with too many signers
    let mut qc2 = qc.clone();
    qc2.signers = Signers::new(setup.genesis.validators_schedule.as_ref().unwrap().len() + 1);
    assert_matches!(
        qc2.verify(&setup.genesis),
        Err(CommitQCVerifyError::BadSignersSet)
    );

    // QC with not enough weight
    let mut qc3 = qc.clone();
    qc3.signers.0.set(0, false);
    qc3.signers.0.set(4, false);
    assert_matches!(
        qc3.verify(&setup.genesis),
        Err(CommitQCVerifyError::NotEnoughWeight { .. })
    );

    // QC with bad signature
    let mut qc4 = qc.clone();
    qc4.signature = rng.gen();
    assert_matches!(
        qc4.verify(&setup.genesis),
        Err(CommitQCVerifyError::BadSignature(_))
    );
}
