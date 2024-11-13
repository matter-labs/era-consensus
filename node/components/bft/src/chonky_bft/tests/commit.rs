use crate::chonky_bft::{commit, testonly::UTHarness};
use assert_matches::assert_matches;
use pretty_assertions::assert_eq;
use rand::Rng;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator;

#[tokio::test]
async fn commit_yield_new_view_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let cur_view = util.replica.view_number;
        let replica_commit = util.new_replica_commit(ctx).await;
        assert_eq!(util.replica.phase, validator::Phase::Commit);

        let new_view = util
            .process_replica_commit_all(ctx, replica_commit.clone())
            .await
            .msg;
        assert_eq!(util.replica.view_number, cur_view.next());
        assert_eq!(util.replica.phase, validator::Phase::Prepare);
        assert_eq!(new_view.view().number, cur_view.next());
        assert_matches!(new_view.justification, validator::ProposalJustification::Commit(qc) => {
            assert_eq!(qc.message.proposal, replica_commit.proposal);
        });

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn commit_non_validator_signer() {
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
            Err(commit::Error::NonValidatorSigner { signer }) => {
                assert_eq!(*signer, non_validator_key.public());
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
        replica_commit.view.number = validator::ViewNumber(util.replica.view_number.0 - 1);
        let res = util
            .process_replica_commit(ctx, util.owner_key().sign_msg(replica_commit))
            .await;

        assert_matches!(
            res,
            Err(commit::Error::Old { current_view }) => {
                assert_eq!(current_view, util.replica.view_number);
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn commit_duplicate_signer() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_commit = util.new_replica_commit(ctx).await;
        assert!(util
            .process_replica_commit(ctx, util.owner_key().sign_msg(replica_commit.clone()))
            .await
            .unwrap()
            .is_none());

        // Processing twice same ReplicaCommit for same view gets DuplicateSigner error
        let res = util
            .process_replica_commit(ctx, util.owner_key().sign_msg(replica_commit.clone()))
            .await;
        assert_matches!(
            res,
            Err(commit::Error::DuplicateSigner {
                message_view,
                signer
            })=> {
                assert_eq!(message_view, util.replica.view_number);
                assert_eq!(*signer, util.owner_key().public());
            }
        );

        // Processing twice different ReplicaCommit for same view gets DuplicateSigner error too
        replica_commit.proposal.number = replica_commit.proposal.number.next();
        let res = util
            .process_replica_commit(ctx, util.owner_key().sign_msg(replica_commit.clone()))
            .await;
        assert_matches!(
            res,
            Err(commit::Error::DuplicateSigner {
                message_view,
                signer
            })=> {
                assert_eq!(message_view, util.replica.view_number);
                assert_eq!(*signer, util.owner_key().public());
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn commit_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let msg = util.new_replica_commit(ctx).await;
        let mut replica_commit = util.owner_key().sign_msg(msg);
        replica_commit.sig = ctx.rng().gen();

        let res = util.process_replica_commit(ctx, replica_commit).await;
        assert_matches!(res, Err(commit::Error::InvalidSignature(..)));

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn commit_invalid_message() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_commit = util.new_replica_commit(ctx).await;
        replica_commit.view.genesis = ctx.rng().gen();

        let res = util
            .process_replica_commit(ctx, util.owner_key().sign_msg(replica_commit))
            .await;
        assert_matches!(res, Err(commit::Error::InvalidMessage(_)));

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
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_replica_commit(ctx).await;
        for i in 0..util.genesis().validators.quorum_threshold() as usize - 1 {
            assert!(util
                .process_replica_commit(ctx, util.keys[i].sign_msg(replica_commit.clone()))
                .await
                .unwrap()
                .is_none());
        }
        let res = util
            .process_replica_commit(
                ctx,
                util.keys[util.genesis().validators.quorum_threshold() as usize - 1]
                    .sign_msg(replica_commit.clone()),
            )
            .await
            .unwrap()
            .unwrap()
            .msg;
        assert_matches!(res.justification, validator::ProposalJustification::Commit(qc) => {
            assert_eq!(qc.message.proposal, replica_commit.proposal);
        });
        for i in util.genesis().validators.quorum_threshold() as usize..util.keys.len() {
            let res = util
                .process_replica_commit(ctx, util.keys[i].sign_msg(replica_commit.clone()))
                .await;
            assert_matches!(res, Err(commit::Error::Old { .. }));
        }

        Ok(())
    })
    .await
    .unwrap();
}

/// ReplicaCommit received before receiving LeaderProposal.
/// Whether replica accepts or rejects the message it doesn't matter.
/// It just shouldn't crash.
#[tokio::test]
async fn replica_commit_unexpected_proposal() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        util.produce_block(ctx).await;
        let replica_commit = validator::ReplicaCommit {
            view: util.view(),
            proposal: validator::BlockHeader {
                number: util
                    .replica
                    .high_commit_qc
                    .as_ref()
                    .unwrap()
                    .message
                    .proposal
                    .number
                    .next(),
                payload: ctx.rng().gen(),
            },
        };

        let _ = util
            .process_replica_commit(ctx, util.owner_key().sign_msg(replica_commit))
            .await;

        Ok(())
    })
    .await
    .unwrap();
}

/// Proposal should be the same for every ReplicaCommit
/// Check it doesn't fail if one validator sends a different proposal in
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
        util.process_replica_commit(ctx, util.owner_key().sign_msg(bad_replica_commit))
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
        assert_matches!(replica_commit_result.unwrap().msg.justification, validator::ProposalJustification::Commit(qc) => {
            assert_eq!(qc.message.proposal, replica_commit.proposal);
        });

        Ok(())
    })
    .await
    .unwrap();
}

/// Check that leader won't accumulate undefined amount of messages if
/// it's spammed with ReplicaCommit messages for future views
#[tokio::test]
async fn replica_commit_limit_messages_in_memory() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_commit = util.new_replica_commit(ctx).await;
        let mut view = util.view();
        // Spam it with 200 messages for different views
        for _ in 0..200 {
            replica_commit.view = view;
            let res = util
                .process_replica_commit(ctx, util.owner_key().sign_msg(replica_commit.clone()))
                .await;
            assert_matches!(res, Ok(_));
            view.number = view.number.next();
        }

        // Ensure only 1 commit_qc is in memory, as the previous 199 were discarded each time
        // a new message was processed
        assert_eq!(util.replica.commit_qcs_cache.len(), 1);

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_commit_filter_functions_test() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_replica_commit(ctx).await;
        let msg = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaCommit(
                replica_commit.clone(),
            ));

        // Send a msg with invalid signature
        let mut invalid_msg = msg.clone();
        invalid_msg.sig = ctx.rng().gen();
        util.send(invalid_msg);

        // Send a correct message
        util.send(msg.clone());

        // Validate only correct message is received
        assert_eq!(
            util.replica.inbound_channel.recv(ctx).await.unwrap().msg,
            msg
        );

        // Send a msg with view number = 2
        let mut replica_commit_from_view_2 = replica_commit.clone();
        replica_commit_from_view_2.view.number = validator::ViewNumber(2);
        let msg_from_view_2 = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaCommit(
                replica_commit_from_view_2,
            ));
        util.send(msg_from_view_2);

        // Send a msg with view number = 4, will prune message from view 2
        let mut replica_commit_from_view_4 = replica_commit.clone();
        replica_commit_from_view_4.view.number = validator::ViewNumber(4);
        let msg_from_view_4 = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaCommit(
                replica_commit_from_view_4,
            ));
        util.send(msg_from_view_4.clone());

        // Send a msg with view number = 3, will be discarded, as it is older than message from view 4
        let mut replica_commit_from_view_3 = replica_commit.clone();
        replica_commit_from_view_3.view.number = validator::ViewNumber(3);
        let msg_from_view_3 = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaCommit(
                replica_commit_from_view_3,
            ));
        util.send(msg_from_view_3);

        // Validate only message from view 4 is received
        assert_eq!(
            util.replica.inbound_channel.recv(ctx).await.unwrap().msg,
            msg_from_view_4
        );

        // Send a msg from validator 0
        let msg_from_validator_0 = util.keys[0].sign_msg(validator::ConsensusMsg::ReplicaCommit(
            replica_commit.clone(),
        ));
        util.send(msg_from_validator_0.clone());

        // Send a msg from validator 1
        let msg_from_validator_1 = util.keys[1].sign_msg(validator::ConsensusMsg::ReplicaCommit(
            replica_commit.clone(),
        ));
        util.send(msg_from_validator_1.clone());

        //Validate both are present in the inbound_channel.
        assert_eq!(
            util.replica.inbound_channel.recv(ctx).await.unwrap().msg,
            msg_from_validator_0
        );
        assert_eq!(
            util.replica.inbound_channel.recv(ctx).await.unwrap().msg,
            msg_from_validator_1
        );

        Ok(())
    })
    .await
    .unwrap();
}
