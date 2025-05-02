use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_engine::testonly::in_memory;
use zksync_consensus_roles::validator;

use crate::v1_chonky_bft::{
    proposal,
    testonly::{UnitTestHarness, MAX_PAYLOAD_SIZE},
};

#[tokio::test]
async fn proposal_yield_replica_commit_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let proposal = util.new_leader_proposal(ctx).await;
        let replica_commit = util
            .process_leader_proposal(ctx, util.owner_key().sign_msg(proposal.clone()))
            .await
            .unwrap();

        assert_eq!(
            replica_commit.msg,
            validator::v1::ReplicaCommit {
                view: proposal.view(),
                proposal: validator::v1::BlockHeader {
                    number: proposal
                        .justification
                        .get_implied_block(util.validators(), util.first_block())
                        .0,
                    payload: proposal.proposal_payload.unwrap().hash()
                },
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_old_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let proposal = util.new_leader_proposal(ctx).await;

        util.replica.phase = validator::v1::Phase::Commit;

        let res = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal.clone()))
            .await;

        assert_matches!(
            res,
            Err(proposal::Error::Old { current_view, current_phase }) => {
                assert_eq!(current_view, util.replica.view_number);
                assert_eq!(current_phase, util.replica.phase);
            }
        );

        util.replica.phase = validator::v1::Phase::Timeout;

        let res = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal.clone()))
            .await;

        assert_matches!(
            res,
            Err(proposal::Error::Old { current_view, current_phase }) => {
                assert_eq!(current_view, util.replica.view_number);
                assert_eq!(current_phase, util.replica.phase);
            }
        );

        util.replica.phase = validator::v1::Phase::Prepare;
        util.replica.view_number = util.replica.view_number.next();

        let res = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal))
            .await;

        assert_matches!(
            res,
            Err(proposal::Error::Old { current_view, current_phase }) => {
                assert_eq!(current_view, util.replica.view_number);
                assert_eq!(current_phase, util.replica.phase);
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_invalid_leader() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let proposal = util.new_leader_proposal(ctx).await;

        assert_ne!(
            util.view_leader(proposal.view().number),
            util.owner_key().public()
        );

        let res = util
            .process_leader_proposal(ctx, util.owner_key().sign_msg(proposal))
            .await;

        assert_matches!(
            res,
            Err(proposal::Error::InvalidLeader { correct_leader, received_leader }) => {
                assert_eq!(correct_leader, util.keys[1].public());
                assert_eq!(received_leader, util.keys[0].public());
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_invalid_signature() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let proposal = util.new_leader_proposal(ctx).await;
        let mut signed_proposal = util.leader_key().sign_msg(proposal);
        signed_proposal.sig = ctx.rng().gen();

        let res = util.process_leader_proposal(ctx, signed_proposal).await;

        assert_matches!(res, Err(proposal::Error::InvalidSignature(_)));

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_invalid_message() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut proposal = util.new_leader_proposal(ctx).await;
        proposal.justification = ctx.rng().gen();
        let res = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal))
            .await;

        assert_matches!(res, Err(proposal::Error::InvalidMessage(_)));

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_pruned_block() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let fake_commit = validator::v1::ReplicaCommit {
            view: util.view(),
            proposal: validator::v1::BlockHeader {
                number: util
                    .replica
                    .config
                    .engine_manager
                    .queued()
                    .first
                    .prev()
                    .unwrap()
                    .prev()
                    .unwrap(),
                payload: ctx.rng().gen(),
            },
        };

        util.process_replica_commit_all(ctx, fake_commit).await;

        // The replica should now produce a proposal for an already pruned block number.
        let proposal = util.new_leader_proposal(ctx).await;

        let res = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal))
            .await;

        assert_matches!(res, Err(proposal::Error::ProposalAlreadyPruned));

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_reproposal_with_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        util.new_replica_commit(ctx).await;
        let replica_timeout = util.new_replica_timeout(ctx).await;
        util.process_replica_timeout_all(ctx, replica_timeout).await;

        let mut proposal = util.new_leader_proposal(ctx).await;
        assert!(proposal.proposal_payload.is_none());
        proposal.proposal_payload = Some(ctx.rng().gen());

        let res = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal))
            .await;

        assert_matches!(res, Err(proposal::Error::ReproposalWithPayload));

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_missing_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut proposal = util.new_leader_proposal(ctx).await;
        proposal.proposal_payload = None;

        let res = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal))
            .await;

        assert_matches!(res, Err(proposal::Error::MissingPayload));

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_proposal_oversized_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let payload = validator::Payload(vec![0; MAX_PAYLOAD_SIZE + 1]);
        let mut proposal = util.new_leader_proposal(ctx).await;
        proposal.proposal_payload = Some(payload);

        let res = util
            .process_leader_proposal(ctx, util.owner_key().sign_msg(proposal))
            .await;
        assert_matches!(
            res,
            Err(proposal::Error::ProposalOversizedPayload{ payload_size }) => {
                assert_eq!(payload_size, MAX_PAYLOAD_SIZE + 1);
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_missing_previous_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UnitTestHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let missing_payload_number = util.replica.config.engine_manager.queued().first.next();
        let fake_commit = validator::v1::ReplicaCommit {
            view: util.view(),
            proposal: validator::v1::BlockHeader {
                number: missing_payload_number,
                payload: ctx.rng().gen(),
            },
        };

        util.process_replica_commit_all(ctx, fake_commit).await;

        let proposal = validator::v1::LeaderProposal {
            proposal_payload: Some(ctx.rng().gen()),
            justification: validator::v1::ProposalJustification::Commit(
                util.replica.high_commit_qc.clone().unwrap(),
            ),
        };

        let res = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal))
            .await;

        assert_matches!(
            res,
            Err(proposal::Error::MissingPreviousPayload { prev_number } ) => {
                assert_eq!(prev_number, missing_payload_number);
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proposal_invalid_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) =
            UnitTestHarness::new_with_payload_manager(ctx, 1, in_memory::PayloadManager::Reject)
                .await;
        s.spawn_bg(runner.run(ctx));

        let proposal = util.new_leader_proposal(ctx).await;

        let res = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal))
            .await;

        assert_matches!(res, Err(proposal::Error::InvalidPayload(_)));

        Ok(())
    })
    .await
    .unwrap();
}
