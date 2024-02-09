use super::{
    replica_commit::Error as ReplicaCommitError, replica_prepare::Error as ReplicaPrepareError,
};
use crate::testonly::ut_harness::UTHarness;
use assert_matches::assert_matches;
use pretty_assertions::assert_eq;
use rand::Rng;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator::{self, Phase, ViewNumber};

#[tokio::test]
async fn replica_prepare_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.new_leader_prepare(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_sanity_yield_leader_prepare() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_prepare = util.new_replica_prepare(|_| {});
        let leader_prepare = util
            .process_replica_prepare(ctx, replica_prepare.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(leader_prepare.msg.view(), &replica_prepare.msg.view);
        assert_eq!(
            leader_prepare.msg.proposal.parent,
            replica_prepare.msg.high_vote.map(|v|v.proposal.hash()),
        );
        assert_eq!(
            leader_prepare.msg.justification,
            util.new_prepare_qc(|msg| *msg = replica_prepare.msg)
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_sanity_yield_leader_prepare_reproposal() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.new_replica_commit(ctx).await;
        util.process_replica_timeout(ctx).await;
        let replica_prepare = util.new_replica_prepare(|_| {}).msg;
        let leader_prepare = util
            .process_replica_prepare_all(ctx, replica_prepare.clone())
            .await;

        assert_eq!(leader_prepare.msg.view(), &replica_prepare.view);
        assert_eq!(
            Some(leader_prepare.msg.proposal),
            replica_prepare.high_vote.map(|v|v.proposal),
        );
        assert_eq!(leader_prepare.msg.proposal_payload, None);
        let map = leader_prepare.msg.justification.map;
        assert_eq!(map.len(), 1);
        assert_eq!(*map.first_key_value().unwrap().0, replica_prepare);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_incompatible_protocol_version() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx,s| async {
        let (mut util,runner) = UTHarness::new(ctx,1).await;
        s.spawn_bg(runner.run(ctx));

        let incompatible_protocol_version = util.incompatible_protocol_version();
        let replica_prepare = util.new_replica_prepare(|msg| {
            msg.view.protocol_version = incompatible_protocol_version;
        });
        let res = util.process_replica_prepare(ctx, replica_prepare).await;
        assert_matches!(
            res,
            Err(ReplicaPrepareError::IncompatibleProtocolVersion { message_version, local_version }) => {
                assert_eq!(message_version, incompatible_protocol_version);
                assert_eq!(local_version, util.protocol_version());
            }
        );
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn replica_prepare_non_validator_signer() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_prepare = util.new_replica_prepare(|_| {}).msg;
        let non_validator_key: validator::SecretKey = ctx.rng().gen();
        let res = util
            .process_replica_prepare(ctx, non_validator_key.sign_msg(replica_prepare))
            .await;
        assert_matches!(
            res,
            Err(ReplicaPrepareError::NonValidatorSigner { signer }) => {
                assert_eq!(signer, non_validator_key.public());
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_old_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_prepare = util.new_replica_prepare(|_| {});
        util.leader.view = util.replica.view.next();
        util.leader.phase = Phase::Prepare;
        let res = util.process_replica_prepare(ctx, replica_prepare).await;
        assert_matches!(
            res,
            Err(ReplicaPrepareError::Old {
                current_view: ViewNumber(2),
                current_phase: Phase::Prepare,
            })
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_during_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_prepare = util.new_replica_prepare(|_| {});
        util.leader.view = util.replica.view;
        util.leader.phase = Phase::Commit;
        let res = util.process_replica_prepare(ctx, replica_prepare).await;
        assert_matches!(
            res,
            Err(ReplicaPrepareError::Old {
                current_view,
                current_phase: Phase::Commit,
            }) => {
                assert_eq!(current_view, util.replica.view);
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_not_leader_in_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let replica_prepare = util.new_replica_prepare(|msg| {
            // Moving to the next view changes the leader.
            msg.view.number = msg.view.number.next();
        });
        let res = util.process_replica_prepare(ctx, replica_prepare).await;
        assert_matches!(res, Err(ReplicaPrepareError::NotLeaderInView));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_already_exists() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        util.set_owner_as_view_leader();
        let replica_prepare = util.new_replica_prepare(|_| {});
        assert!(util
            .process_replica_prepare(ctx, replica_prepare.clone())
            .await
            .unwrap()
            .is_none());
        let res = util
            .process_replica_prepare(ctx, replica_prepare.clone())
            .await;
        assert_matches!(
            res,
            Err(ReplicaPrepareError::Exists { existing_message }) => {
                assert_eq!(existing_message, replica_prepare.msg);
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_num_received_below_threshold() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        util.set_owner_as_view_leader();
        let replica_prepare = util.new_replica_prepare(|_| {});
        assert!(util
            .process_replica_prepare(ctx, replica_prepare)
            .await
            .unwrap()
            .is_none());
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_prepare = util.new_replica_prepare(|_| {});
        replica_prepare.sig = ctx.rng().gen();
        let res = util.process_replica_prepare(ctx, replica_prepare).await;
        assert_matches!(res, Err(ReplicaPrepareError::InvalidSignature(_)));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_invalid_commit_qc() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_prepare = util.new_replica_prepare(|msg| msg.high_qc = ctx.rng().gen());
        let res = util.process_replica_prepare(ctx, replica_prepare).await;
        // TODO: assert_matches!(res, Err(ReplicaPrepareError::InvalidHighQC(..)));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_high_qc_of_current_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let view = ViewNumber(1);
        let qc_view = ViewNumber(1);
        util.set_view(view);
        let qc = util.new_commit_qc(|msg| msg.view.number = qc_view);
        let replica_prepare = util.new_replica_prepare(|msg| msg.high_qc = Some(qc));
        let res = util.process_replica_prepare(ctx, replica_prepare).await;
        /*assert_matches!(
            res,
            Err(ReplicaPrepareError::HighQCOfFutureView { high_qc_view, current_view }) => {
                assert_eq!(high_qc_view, qc_view);
                assert_eq!(current_view, view);
            }
        );*/
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_high_qc_of_future_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let view = ViewNumber(1);
        let qc_view = ViewNumber(2);
        util.set_view(view);
        let qc = util.new_commit_qc(|msg| msg.view.number = qc_view);
        let replica_prepare = util.new_replica_prepare(|msg| msg.high_qc = Some(qc));
        let res = util.process_replica_prepare(ctx, replica_prepare).await;
        /*TODO: assert_matches!(
            res,
            Err(ReplicaPrepareError::HighQCOfFutureView{ high_qc_view, current_view }) => {
                assert_eq!(high_qc_view, qc_view);
                assert_eq!(current_view, view);
            }
        );*/
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.new_leader_commit(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_sanity_yield_leader_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_replica_commit(ctx).await;
        let leader_commit = util
            .process_replica_commit(ctx, replica_commit.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(leader_commit.msg.justification, util.new_commit_qc(|msg| *msg = replica_commit.msg));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_incompatible_protocol_version() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx,s| async {
        let (mut util,runner) = UTHarness::new(ctx,1).await;
        s.spawn_bg(runner.run(ctx));

        let incompatible_protocol_version = util.incompatible_protocol_version();
        let mut replica_commit = util.new_replica_commit(ctx).await.msg;
        replica_commit.view.protocol_version = incompatible_protocol_version;
        let res = util
            .process_replica_commit(ctx, util.owner_key().sign_msg(replica_commit))
            .await;
        assert_matches!(
            res,
            Err(ReplicaCommitError::IncompatibleProtocolVersion { message_version, local_version }) => {
                assert_eq!(message_version, incompatible_protocol_version);
                assert_eq!(local_version, util.protocol_version());
            }
        );
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn replica_commit_non_validator_signer() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_replica_commit(ctx).await.msg;
        let non_validator_key: validator::SecretKey = ctx.rng().gen();
        let res = util
            .process_replica_commit(ctx, non_validator_key.sign_msg(replica_commit))
            .await;
        assert_matches!(
            res,
            Err(ReplicaCommitError::NonValidatorSigner { signer }) => {
                assert_eq!(signer, non_validator_key.public());
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_old() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_commit = util.new_replica_commit(ctx).await.msg;
        replica_commit.view.number = util.replica.view.prev();
        let replica_commit = util.owner_key().sign_msg(replica_commit);
        let res = util.process_replica_commit(ctx, replica_commit).await;
        assert_matches!(
            res,
            Err(ReplicaCommitError::Old { current_view, current_phase }) => {
                assert_eq!(current_view, util.replica.view);
                assert_eq!(current_phase, util.replica.phase);
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_not_leader_in_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let current_view_leader = util.view_leader(util.replica.view);
        assert_ne!(current_view_leader, util.owner_key().public());

        let replica_commit = util.new_current_replica_commit(|_| {});
        let res = util.process_replica_commit(ctx, replica_commit).await;
        assert_matches!(res, Err(ReplicaCommitError::NotLeaderInView));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_already_exists() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_replica_commit(ctx).await;
        assert!(util
            .process_replica_commit(ctx, replica_commit.clone())
            .await
            .unwrap()
            .is_none());
        let res = util
            .process_replica_commit(ctx, replica_commit.clone())
            .await;
        assert_matches!(
            res,
            Err(ReplicaCommitError::DuplicateMessage { existing_message }) => {
                assert_eq!(existing_message, replica_commit.msg)
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_num_received_below_threshold() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let replica_prepare = util.new_replica_prepare(|_| {});
        assert!(util
            .process_replica_prepare(ctx, replica_prepare.clone())
            .await
            .unwrap()
            .is_none());
        let replica_prepare = util.keys[1].sign_msg(replica_prepare.msg);
        let leader_prepare = util
            .process_replica_prepare(ctx, replica_prepare)
            .await
            .unwrap()
            .unwrap();
        let replica_commit = util
            .process_leader_prepare(ctx, leader_prepare)
            .await
            .unwrap();
        util.process_replica_commit(ctx, replica_commit.clone())
            .await
            .unwrap();
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_commit = util.new_current_replica_commit(|_| {});
        replica_commit.sig = ctx.rng().gen();
        let res = util.process_replica_commit(ctx, replica_commit).await;
        assert_matches!(res, Err(ReplicaCommitError::InvalidSignature(..)));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_unexpected_proposal() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_current_replica_commit(|_| {});
        util.process_replica_commit(ctx, replica_commit)
            .await
            .unwrap();
        Ok(())
    })
    .await
    .unwrap();
}
