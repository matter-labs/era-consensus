use super::*;
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
        tracing::info!("started");
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

        util.produce_block(ctx).await;
        let replica_prepare = util.new_replica_prepare();
        let leader_prepare = util
            .process_replica_prepare(ctx, util.sign(replica_prepare.clone()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(leader_prepare.msg.view(), &replica_prepare.view);
        assert_eq!(
            leader_prepare.msg.justification,
            util.new_prepare_qc(|msg| *msg = replica_prepare)
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
        let replica_prepare = util.new_replica_prepare();
        let leader_prepare = util
            .process_replica_prepare_all(ctx, replica_prepare.clone())
            .await;

        assert_eq!(leader_prepare.msg.view(), &replica_prepare.view);
        assert_eq!(
            Some(leader_prepare.msg.proposal),
            replica_prepare.high_vote.as_ref().map(|v| v.proposal),
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
        let mut replica_prepare = util.new_replica_prepare();
        replica_prepare.view.protocol_version = incompatible_protocol_version;
        let res = util.process_replica_prepare(ctx, util.sign(replica_prepare)).await;
        assert_matches!(
            res,
            Err(replica_prepare::Error::IncompatibleProtocolVersion { message_version, local_version }) => {
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

        let replica_prepare = util.new_replica_prepare();
        let non_validator_key: validator::SecretKey = ctx.rng().gen();
        let res = util
            .process_replica_prepare(ctx, non_validator_key.sign_msg(replica_prepare))
            .await;
        assert_matches!(
            res,
            Err(replica_prepare::Error::NonValidatorSigner { signer }) => {
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

        let replica_prepare = util.new_replica_prepare();
        util.leader.view = util.replica.view.next();
        util.leader.phase = Phase::Prepare;
        let res = util
            .process_replica_prepare(ctx, util.sign(replica_prepare))
            .await;
        assert_matches!(
            res,
            Err(replica_prepare::Error::Old {
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

        let replica_prepare = util.new_replica_prepare();
        util.leader.view = util.replica.view;
        util.leader.phase = Phase::Commit;
        let res = util
            .process_replica_prepare(ctx, util.sign(replica_prepare))
            .await;
        assert_matches!(
            res,
            Err(replica_prepare::Error::Old {
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

        let mut replica_prepare = util.new_replica_prepare();
        replica_prepare.view.number = replica_prepare.view.number.next();
        let res = util
            .process_replica_prepare(ctx, util.sign(replica_prepare))
            .await;
        assert_matches!(res, Err(replica_prepare::Error::NotLeaderInView));
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
        let replica_prepare = util.new_replica_prepare();
        let replica_prepare = util.sign(replica_prepare.clone());
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
            Err(replica_prepare::Error::Exists { existing_message }) => {
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
        let replica_prepare = util.new_replica_prepare();
        assert!(util
            .process_replica_prepare(ctx, util.sign(replica_prepare))
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

        let msg = util.new_replica_prepare();
        let mut replica_prepare = util.sign(msg);
        replica_prepare.sig = ctx.rng().gen();
        let res = util.process_replica_prepare(ctx, replica_prepare).await;
        assert_matches!(res, Err(replica_prepare::Error::InvalidSignature(_)));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_invalid_commit_qc() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        util.produce_block(ctx).await;
        let mut replica_prepare = util.new_replica_prepare();
        replica_prepare.high_qc.as_mut().unwrap().signature = rng.gen();
        let res = util
            .process_replica_prepare(ctx, util.sign(replica_prepare))
            .await;
        assert_matches!(
            res,
            Err(replica_prepare::Error::InvalidMessage(
                validator::ReplicaPrepareVerifyError::HighQC(_)
            ))
        );
        Ok(())
    })
    .await
    .unwrap();
}

/// Check that leader behaves correctly in case receiving ReplicaPrepare
/// with high_qc with future views (which shouldn't be available yet).
#[tokio::test]
async fn replica_prepare_high_qc_of_future_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        util.produce_block(ctx).await;
        let mut view = util.replica_view();
        let mut replica_prepare = util.new_replica_prepare();
        // Check both the current view and next view.
        for _ in 0..2 {
            let qc = util.new_commit_qc(|msg| msg.view = view.clone());
            replica_prepare.high_qc = Some(qc);
            let res = util
                .process_replica_prepare(ctx, util.sign(replica_prepare.clone()))
                .await;
            assert_matches!(
                res,
                Err(replica_prepare::Error::InvalidMessage(
                    validator::ReplicaPrepareVerifyError::HighQCFutureView
                ))
            );
            view.number = view.number.next();
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Check all ReplicaPrepare are included for weight calculation
/// even on different messages for the same view.
#[tokio::test]
async fn replica_prepare_different_messages() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.produce_block(ctx).await;

        let view = util.replica_view();
        let replica_prepare = util.new_replica_prepare();

        // Create a different proposal for the same view
        let proposal = replica_prepare.clone().high_vote.unwrap().proposal;
        let mut different_proposal = proposal;
        different_proposal.number = different_proposal.number.next();

        // Create a new ReplicaPrepare with the different proposal
        let mut other_replica_prepare = replica_prepare.clone();
        let mut high_vote = other_replica_prepare.high_vote.clone().unwrap();
        high_vote.proposal = different_proposal;
        let high_qc = util.new_commit_qc(|msg| {
            msg.proposal = different_proposal;
            msg.view = view.clone()
        });

        other_replica_prepare.high_vote = Some(high_vote);
        other_replica_prepare.high_qc = Some(high_qc);

        let validators = util.keys.len();

        // half of the validators sign replica_prepare
        for i in 0..validators / 2 {
            util.process_replica_prepare(ctx, util.keys[i].sign_msg(replica_prepare.clone()))
                .await
                .unwrap();
        }

        let mut replica_commit_result = None;
        // The rest (minus one) of the validators sign other_replica_prepare
        for i in validators / 2..validators - 1 {
            replica_commit_result = util
                .process_replica_prepare(ctx, util.keys[i].sign_msg(other_replica_prepare.clone()))
                .await
                .unwrap();
        }

        // That should be enough for a proposal to be committed
        assert_matches!(replica_commit_result, Some(_));

        // Check the first proposal has been committed (as it has more votes)
        let message = replica_commit_result.unwrap().msg;
        assert_eq!(message.proposal, proposal);
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

        util.produce_block(ctx).await;
        let replica_commit = util.new_replica_commit(ctx).await;
        let leader_commit = util
            .process_replica_commit(ctx, util.sign(replica_commit.clone()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            leader_commit.msg.justification,
            util.new_commit_qc(|msg| *msg = replica_commit)
        );
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
        let mut replica_commit = util.new_replica_commit(ctx).await;
        replica_commit.view.protocol_version = incompatible_protocol_version;
        let res = util
            .process_replica_commit(ctx, util.sign(replica_commit))
            .await;
        assert_matches!(
            res,
            Err(replica_commit::Error::IncompatibleProtocolVersion { message_version, local_version }) => {
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

        let replica_commit = util.new_replica_commit(ctx).await;
        let non_validator_key: validator::SecretKey = ctx.rng().gen();
        let res = util
            .process_replica_commit(ctx, non_validator_key.sign_msg(replica_commit))
            .await;
        assert_matches!(
            res,
            Err(replica_commit::Error::NonValidatorSigner { signer }) => {
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

        let mut replica_commit = util.new_replica_commit(ctx).await;
        replica_commit.view.number = ViewNumber(util.replica.view.0 - 1);
        let replica_commit = util.sign(replica_commit);
        let res = util.process_replica_commit(ctx, replica_commit).await;
        assert_matches!(
            res,
            Err(replica_commit::Error::Old { current_view, current_phase }) => {
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

        util.produce_block(ctx).await;
        let current_view_leader = util.view_leader(util.replica.view);
        assert_ne!(current_view_leader, util.owner_key().public());
        let replica_commit = util.new_current_replica_commit();
        let res = util
            .process_replica_commit(ctx, util.sign(replica_commit))
            .await;
        assert_matches!(res, Err(replica_commit::Error::NotLeaderInView));
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
            .process_replica_commit(ctx, util.sign(replica_commit.clone()))
            .await
            .unwrap()
            .is_none());
        let res = util
            .process_replica_commit(ctx, util.sign(replica_commit.clone()))
            .await;
        assert_matches!(
            res,
            Err(replica_commit::Error::DuplicateMessage { existing_message }) => {
                assert_eq!(existing_message, replica_commit)
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

        let replica_prepare = util.new_replica_prepare();
        assert!(util
            .process_replica_prepare(ctx, util.sign(replica_prepare.clone()))
            .await
            .unwrap()
            .is_none());
        let replica_prepare = util.keys[1].sign_msg(replica_prepare);
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

        let msg = util.new_replica_commit(ctx).await;
        let mut replica_commit = util.sign(msg);
        replica_commit.sig = ctx.rng().gen();
        let res = util.process_replica_commit(ctx, replica_commit).await;
        assert_matches!(res, Err(replica_commit::Error::InvalidSignature(..)));
        Ok(())
    })
    .await
    .unwrap();
}

/// ReplicaCommit received before sending out LeaderPrepare.
/// Whether leader accepts the message or rejects doesn't matter.
/// It just shouldn't crash.
#[tokio::test]
async fn replica_commit_unexpected_proposal() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        util.produce_block(ctx).await;
        let replica_commit = util.new_current_replica_commit();
        let _ = util
            .process_replica_commit(ctx, util.sign(replica_commit))
            .await;
        Ok(())
    })
    .await
    .unwrap();
}

/// Proposal should be the same for every ReplicaCommit
/// Check it doesn't fail if one validator sends a different propsal in
/// the ReplicaCommit
#[tokio::test]
async fn replica_commit_different_proposals() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_replica_commit(ctx).await;

        // Process a modified replica_commit (ie. from a malicious or wrong node)
        let mut bad_replica_commit = replica_commit.clone();
        bad_replica_commit.proposal.number = replica_commit.proposal.number.next();
        util.process_replica_commit(ctx, util.sign(bad_replica_commit))
            .await
            .unwrap();

        // The rest of the validators sign the correct one
        let mut replica_commit_result = None;
        for i in 1..util.keys.len() {
            replica_commit_result = util
                .process_replica_commit(ctx, util.keys[i].sign_msg(replica_commit.clone()))
                .await
                .unwrap();
        }

        // Check correct proposal has been committed
        assert_matches!(
            replica_commit_result,
            Some(leader_commit) => {
                assert_eq!(
                    leader_commit.msg.justification.message.proposal,
                    replica_commit.proposal
                );
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}
