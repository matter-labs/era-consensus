use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator;

use crate::v2_chonky_bft::{new_view, testonly::UnitTestHarness};

#[tokio::test]
async fn new_view_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let commit_1 = validator::v2::ReplicaCommit {
            view: util.view().next_view(),
            proposal: validator::v2::BlockHeader {
                number: validator::BlockNumber(1),
                payload: ctx.rng().gen(),
            },
        };
        let mut commit_qc_1 = validator::v2::CommitQC::new(
            commit_1.clone(),
            util.genesis().validators_schedule.as_ref().unwrap(),
        );
        for key in &util.keys {
            commit_qc_1
                .add(
                    &key.sign_msg(commit_1.clone()),
                    util.genesis().hash(),
                    util.genesis().validators_schedule.as_ref().unwrap(),
                )
                .unwrap();
        }
        let new_view_1 = validator::v2::ReplicaNewView {
            justification: validator::v2::ProposalJustification::Commit(commit_qc_1.clone()),
        };

        let commit_2 = validator::v2::ReplicaCommit {
            view: commit_1.view.next_view(),
            proposal: validator::v2::BlockHeader {
                number: commit_1.proposal.number.next(),
                payload: ctx.rng().gen(),
            },
        };
        let mut commit_qc_2 = validator::v2::CommitQC::new(
            commit_2.clone(),
            util.genesis().validators_schedule.as_ref().unwrap(),
        );
        for key in &util.keys {
            commit_qc_2
                .add(
                    &key.sign_msg(commit_2.clone()),
                    util.genesis().hash(),
                    util.genesis().validators_schedule.as_ref().unwrap(),
                )
                .unwrap();
        }
        let new_view_2 = validator::v2::ReplicaNewView {
            justification: validator::v2::ProposalJustification::Commit(commit_qc_2.clone()),
        };

        let timeout = validator::v2::ReplicaTimeout {
            view: commit_2.view.next_view(),
            high_vote: None,
            high_qc: Some(commit_qc_2.clone()),
        };
        let mut timeout_qc = validator::v2::TimeoutQC::new(timeout.view);
        for key in &util.keys {
            timeout_qc
                .add(
                    &key.sign_msg(timeout.clone()),
                    util.genesis().hash(),
                    util.genesis().validators_schedule.as_ref().unwrap(),
                )
                .unwrap();
        }
        let new_view_3 = validator::v2::ReplicaNewView {
            justification: validator::v2::ProposalJustification::Timeout(timeout_qc.clone()),
        };

        // Check that first new view with commit QC updates the view and high commit QC.
        let res = util
            .process_replica_new_view(ctx, util.owner_key().sign_msg(new_view_1.clone()))
            .await
            .unwrap()
            .unwrap()
            .msg;
        assert_eq!(util.view(), new_view_1.view());
        assert_matches!(res.justification, validator::v2::ProposalJustification::Commit(qc) => {
            assert_eq!(util.replica.high_commit_qc.clone().unwrap(), qc);
        });

        // Check that the third new view with timeout QC updates the view, high timeout QC and high commit QC.
        let res = util
            .process_replica_new_view(ctx, util.owner_key().sign_msg(new_view_3.clone()))
            .await
            .unwrap()
            .unwrap()
            .msg;
        assert_eq!(util.view(), new_view_3.view());
        assert_matches!(res.justification, validator::v2::ProposalJustification::Timeout(qc) => {
            assert_eq!(util.replica.high_timeout_qc.clone().unwrap(), qc);
            assert_eq!(util.replica.high_commit_qc.clone().unwrap(), qc.high_qc().unwrap().clone());
        });

        // Check that the second new view with commit QC is ignored and doesn't affect the state.
        let res = util
            .process_replica_new_view(ctx, util.owner_key().sign_msg(new_view_2.clone()))
            .await;
        assert_eq!(util.view(), new_view_3.view());
        assert_eq!(util.replica.high_timeout_qc.clone().unwrap(), timeout_qc);
        assert_eq!(
            util.replica.high_commit_qc.clone().unwrap(),
            timeout_qc.high_qc().unwrap().clone()
        );
        assert_matches!(
            res,
            Err(new_view::Error::Old { current_view }) => {
                assert_eq!(current_view, util.replica.view_number);
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn new_view_non_validator_signer() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_new_view = util.new_replica_new_view(ctx).await;
        let non_validator_key: validator::SecretKey = ctx.rng().gen();
        let res = util
            .process_replica_new_view(ctx, non_validator_key.sign_msg(replica_new_view))
            .await;

        assert_matches!(
            res,
            Err(new_view::Error::NonValidatorSigner { signer }) => {
                assert_eq!(*signer, non_validator_key.public());
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_new_view_old() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let replica_new_view = util.new_replica_new_view(ctx).await;
        // Sign the messages with non-leader key.
        let replica_new_view = util.keys[1].sign_msg(replica_new_view);

        // Process new_view twice. The second time it shouldn't be accepted.
        util.process_replica_new_view(ctx, replica_new_view.clone())
            .await
            .unwrap();
        let res = util
            .process_replica_new_view(ctx, replica_new_view.clone())
            .await;

        assert_matches!(
            res,
            Err(new_view::Error::Old { current_view }) => {
                assert_eq!(current_view, util.replica.view_number);
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn new_view_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let msg = util.new_replica_new_view(ctx).await;
        let mut replica_new_view = util.owner_key().sign_msg(msg);
        replica_new_view.sig = ctx.rng().gen();

        let res = util.process_replica_new_view(ctx, replica_new_view).await;
        assert_matches!(res, Err(new_view::Error::InvalidSignature(..)));

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn new_view_invalid_message() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let res = util
            .process_replica_new_view(ctx, util.owner_key().sign_msg(ctx.rng().gen()))
            .await;
        assert_matches!(res, Err(new_view::Error::InvalidMessage(_)));

        Ok(())
    })
    .await
    .unwrap();
}
