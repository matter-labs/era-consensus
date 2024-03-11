use super::{leader_commit, leader_prepare};
use crate::{
    testonly,
    testonly::ut_harness::{UTHarness, MAX_PAYLOAD_SIZE},
};
use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator::{
    self, CommitQC, Payload, PrepareQC, ReplicaCommit, ReplicaPrepare,
};

/// Sanity check of the happy path.
#[tokio::test]
async fn block_production() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));
        util.produce_block(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

/// Sanity check of block production with reproposal.
#[tokio::test]
async fn reproposal_block_production() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));
        util.new_leader_commit(ctx).await;
        util.process_replica_timeout(ctx).await;
        util.produce_block(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_incompatible_protocol_version() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx,s| async {
        let (mut util,runner) = UTHarness::new(ctx,1).await;
        s.spawn_bg(runner.run(ctx));

        let incompatible_protocol_version = util.incompatible_protocol_version();
        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        leader_prepare.justification.view.protocol_version = incompatible_protocol_version;
        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::IncompatibleProtocolVersion { message_version, local_version }) => {
                assert_eq!(message_version, incompatible_protocol_version);
                assert_eq!(local_version, util.protocol_version());
            }
        );
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn leader_prepare_sanity_yield_replica_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let leader_prepare = util.new_leader_prepare(ctx).await;
        let replica_commit = util
            .process_leader_prepare(ctx, util.sign(leader_prepare.clone()))
            .await
            .unwrap();
        assert_eq!(
            replica_commit.msg,
            ReplicaCommit {
                view: leader_prepare.view().clone(),
                proposal: leader_prepare.proposal,
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_invalid_leader() {
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
        let mut leader_prepare = util
            .process_replica_prepare(ctx, replica_prepare)
            .await
            .unwrap()
            .unwrap()
            .msg;
        leader_prepare.justification.view.number = leader_prepare.justification.view.number.next();
        assert_ne!(
            util.view_leader(leader_prepare.view().number),
            util.keys[0].public()
        );

        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::InvalidLeader { correct_leader, received_leader }) => {
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
async fn leader_prepare_old_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        leader_prepare.justification.view.number.0 = util.replica.view.0 - 1;
        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::Old { current_view, current_phase }) => {
                assert_eq!(current_view, util.replica.view);
                assert_eq!(current_phase, util.replica.phase);
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

/// Tests that `WriteBlockStore::verify_payload` is applied before signing a vote.
#[tokio::test]
async fn leader_prepare_invalid_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) =
            UTHarness::new_with_payload(ctx, 1, Box::new(testonly::RejectPayload)).await;
        s.spawn_bg(runner.run(ctx));

        let leader_prepare = util.new_leader_prepare(ctx).await;

        // Insert a finalized block to the storage.
        let mut justification = CommitQC::new(
            ReplicaCommit {
                view: util.replica_view(),
                proposal: leader_prepare.proposal,
            },
            util.genesis(),
        );
        justification.add(&util.sign(justification.message.clone()), util.genesis());
        let block = validator::FinalBlock {
            payload: leader_prepare.proposal_payload.clone().unwrap(),
            justification,
        };
        util.replica
            .config
            .block_store
            .queue_block(ctx, block)
            .await
            .unwrap();

        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(res, Err(leader_prepare::Error::ProposalInvalidPayload(..)));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));
        let leader_prepare = util.new_leader_prepare(ctx).await;
        let mut leader_prepare = util.sign(leader_prepare);
        leader_prepare.sig = ctx.rng().gen();
        let res = util.process_leader_prepare(ctx, leader_prepare).await;
        assert_matches!(res, Err(leader_prepare::Error::InvalidSignature(..)));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_invalid_prepare_qc() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        leader_prepare.justification.signature = ctx.rng().gen();
        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::InvalidMessage(
                validator::LeaderPrepareVerifyError::Justification(_)
            ))
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_proposal_oversized_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let payload_oversize = MAX_PAYLOAD_SIZE + 1;
        let payload = Payload(vec![0; payload_oversize]);
        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        leader_prepare.proposal.payload = payload.hash();
        leader_prepare.proposal_payload = Some(payload);
        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::ProposalOversizedPayload{ payload_size }) => {
                assert_eq!(payload_size, payload_oversize);
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_proposal_mismatched_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        leader_prepare.proposal_payload = Some(ctx.rng().gen());
        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::InvalidMessage(
                validator::LeaderPrepareVerifyError::ProposalMismatchedPayload
            ))
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_proposal_when_previous_not_finalized() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        tracing::info!("Execute view without replicas receiving the LeaderCommit.");
        util.new_leader_commit(ctx).await;
        util.process_replica_timeout(ctx).await;
        tracing::info!("Make leader repropose the block.");
        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        tracing::info!("Modify the message to include a new proposal anyway.");
        let payload: Payload = rng.gen();
        leader_prepare.proposal.payload = payload.hash();
        leader_prepare.proposal_payload = Some(payload);
        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::InvalidMessage(
                validator::LeaderPrepareVerifyError::ProposalWhenPreviousNotFinalized
            ))
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_bad_block_number() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    scope::run!(ctx, |ctx,s| async {
        let (mut util,runner) = UTHarness::new(ctx,1).await;
        s.spawn_bg(runner.run(ctx));

        tracing::info!("Produce initial block.");
        util.produce_block(ctx).await;
        tracing::info!("Make leader propose the next block.");
        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        tracing::info!("Modify the proposal.number so that it doesn't match the previous block");
        leader_prepare.proposal.number = rng.gen();
        let res = util.process_leader_prepare(ctx, util.sign(leader_prepare.clone())).await;
        assert_matches!(res, Err(leader_prepare::Error::InvalidMessage(
            validator::LeaderPrepareVerifyError::BadBlockNumber { got, want }
        )) => {
            assert_eq!(want, leader_prepare.justification.high_qc().unwrap().message.proposal.number.next());
            assert_eq!(got, leader_prepare.proposal.number);
        });
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn leader_prepare_reproposal_without_quorum() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        tracing::info!("make leader repropose a block");
        util.new_leader_commit(ctx).await;
        util.process_replica_timeout(ctx).await;
        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        tracing::info!("modify justification, to make reproposal unjustified");
        let mut replica_prepare: ReplicaPrepare = leader_prepare
            .justification
            .map
            .keys()
            .next()
            .unwrap()
            .clone();
        leader_prepare.justification = PrepareQC::new(leader_prepare.justification.view);
        for key in &util.keys {
            replica_prepare.high_vote.as_mut().unwrap().proposal.payload = rng.gen();
            leader_prepare
                .justification
                .add(&key.sign_msg(replica_prepare.clone()), util.genesis());
        }
        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::InvalidMessage(
                validator::LeaderPrepareVerifyError::ReproposalWithoutQuorum
            ))
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_reproposal_when_finalized() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        tracing::info!("Make leader propose a new block");
        util.produce_block(ctx).await;
        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        tracing::info!(
            "Modify the message so that it is actually a reproposal of the previous block"
        );
        leader_prepare.proposal = leader_prepare
            .justification
            .high_qc()
            .unwrap()
            .message
            .proposal;
        leader_prepare.proposal_payload = None;
        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::InvalidMessage(
                validator::LeaderPrepareVerifyError::ReproposalWhenFinalized
            ))
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_prepare_reproposal_invalid_block() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        tracing::info!("Make leader repropose a block.");
        util.new_leader_commit(ctx).await;
        util.process_replica_timeout(ctx).await;
        let mut leader_prepare = util.new_leader_prepare(ctx).await;
        tracing::info!("Make the reproposal different than expected");
        leader_prepare.proposal.payload = rng.gen();
        let res = util
            .process_leader_prepare(ctx, util.sign(leader_prepare))
            .await;
        assert_matches!(
            res,
            Err(leader_prepare::Error::InvalidMessage(
                validator::LeaderPrepareVerifyError::ReproposalBadBlock
            ))
        );
        Ok(())
    })
    .await
    .unwrap();
}

/// Check that replica provides expecte high_vote and high_qc after finalizing a block.
#[tokio::test]
async fn leader_commit_sanity_yield_replica_prepare() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let leader_commit = util.new_leader_commit(ctx).await;
        let replica_prepare = util
            .process_leader_commit(ctx, util.sign(leader_commit.clone()))
            .await
            .unwrap();
        let mut view = leader_commit.justification.message.view.clone();
        view.number = view.number.next();
        assert_eq!(
            replica_prepare.msg,
            ReplicaPrepare {
                view,
                high_vote: Some(leader_commit.justification.message.clone()),
                high_qc: Some(leader_commit.justification),
            }
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_commit_incompatible_protocol_version() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx,s| async {
        let (mut util,runner) = UTHarness::new(ctx,1).await;
        s.spawn_bg(runner.run(ctx));

        let incompatible_protocol_version = util.incompatible_protocol_version();
        let mut leader_commit = util.new_leader_commit(ctx).await;
        leader_commit.justification.message.view.protocol_version = incompatible_protocol_version;
        let res = util.process_leader_commit(ctx, util.sign(leader_commit)).await;
        assert_matches!(
            res,
            Err(leader_commit::Error::IncompatibleProtocolVersion { message_version, local_version }) => {
                assert_eq!(message_version, incompatible_protocol_version);
                assert_eq!(local_version, util.protocol_version());
            }
        );
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn leader_commit_bad_leader() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));
        let leader_commit = util.new_leader_commit(ctx).await;
        // Sign the leader_prepare with a key of different validator.
        let res = util
            .process_leader_commit(ctx, util.keys[1].sign_msg(leader_commit))
            .await;
        assert_matches!(res, Err(leader_commit::Error::BadLeader { .. }));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_commit_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));
        let leader_commit = util.new_leader_commit(ctx).await;
        let mut leader_commit = util.sign(leader_commit);
        leader_commit.sig = rng.gen();
        let res = util.process_leader_commit(ctx, leader_commit).await;
        assert_matches!(res, Err(leader_commit::Error::InvalidSignature { .. }));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn leader_commit_invalid_commit_qc() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut leader_commit = util.new_leader_commit(ctx).await;
        leader_commit.justification.signature = rng.gen();
        let res = util
            .process_leader_commit(ctx, util.sign(leader_commit))
            .await;
        assert_matches!(
            res,
            Err(leader_commit::Error::InvalidMessage(
                validator::CommitQCVerifyError::BadSignature(..)
            ))
        );
        Ok(())
    })
    .await
    .unwrap();
}
