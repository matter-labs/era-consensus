use crate::chonky_bft::{testonly::UTHarness, timeout};
use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator;

#[tokio::test]
async fn timeout_yield_new_view_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let cur_view = util.replica.view_number;
        let replica_timeout = util.new_replica_timeout(ctx).await;
        assert_eq!(util.replica.phase, validator::Phase::Timeout);

        let new_view = util
            .process_replica_timeout_all(ctx, replica_timeout.clone())
            .await
            .msg;
        assert_eq!(util.replica.view_number, cur_view.next());
        assert_eq!(util.replica.phase, validator::Phase::Prepare);
        assert_eq!(new_view.view().number, cur_view.next());

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_non_validator_signer() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_timeout = util.new_replica_timeout(ctx).await;
        let non_validator_key: validator::SecretKey = ctx.rng().gen();
        let res = util
            .process_replica_timeout(ctx, non_validator_key.sign_msg(replica_timeout))
            .await;

        assert_matches!(
            res,
            Err(timeout::Error::NonValidatorSigner { signer }) => {
                assert_eq!(*signer, non_validator_key.public());
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_timeout_old() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_timeout = util.new_replica_timeout(ctx).await;
        replica_timeout.view.number = validator::ViewNumber(util.replica.view_number.0 - 1);
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout))
            .await;

        assert_matches!(
            res,
            Err(timeout::Error::Old { current_view }) => {
                assert_eq!(current_view, util.replica.view_number);
            }
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_duplicate_signer() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        util.produce_block(ctx).await;

        let replica_timeout = util.new_replica_timeout(ctx).await;
        assert!(util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout.clone()))
            .await
            .unwrap()
            .is_none());

        // Processing twice same ReplicaTimeout for same view gets DuplicateSigner error
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout.clone()))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::DuplicateSigner {
                message_view,
                signer
            })=> {
                assert_eq!(message_view, util.replica.view_number);
                assert_eq!(*signer, util.owner_key().public());
            }
        );

        // Processing twice different ReplicaTimeout for same view gets DuplicateSigner error too
        // replica_timeout.high_vote = None;
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout.clone()))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::DuplicateSigner {
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
async fn timeout_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let msg = util.new_replica_timeout(ctx).await;
        let mut replica_timeout = util.owner_key().sign_msg(msg);
        replica_timeout.sig = ctx.rng().gen();

        let res = util.process_replica_timeout(ctx, replica_timeout).await;
        assert_matches!(res, Err(timeout::Error::InvalidSignature(..)));

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_invalid_message() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 1).await;
        s.spawn_bg(runner.run(ctx));

        let replica_timeout = util.new_replica_timeout(ctx).await;

        let mut bad_replica_timeout = replica_timeout.clone();
        bad_replica_timeout.view.genesis = ctx.rng().gen();
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(bad_replica_timeout))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::InvalidMessage(
                validator::ReplicaTimeoutVerifyError::BadView(_)
            ))
        );

        let mut bad_replica_timeout = replica_timeout.clone();
        bad_replica_timeout.high_vote = Some(ctx.rng().gen());
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(bad_replica_timeout))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::InvalidMessage(
                validator::ReplicaTimeoutVerifyError::InvalidHighVote(_)
            ))
        );

        let mut bad_replica_timeout = replica_timeout.clone();
        bad_replica_timeout.high_qc = Some(ctx.rng().gen());
        let res = util
            .process_replica_timeout(ctx, util.owner_key().sign_msg(bad_replica_timeout))
            .await;
        assert_matches!(
            res,
            Err(timeout::Error::InvalidMessage(
                validator::ReplicaTimeoutVerifyError::InvalidHighQC(_)
            ))
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_num_received_below_threshold() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let replica_timeout = util.new_replica_timeout(ctx).await;
        for i in 0..util.genesis().validators.quorum_threshold() as usize - 1 {
            assert!(util
                .process_replica_timeout(ctx, util.keys[i].sign_msg(replica_timeout.clone()))
                .await
                .unwrap()
                .is_none());
        }
        let res = util
            .process_replica_timeout(
                ctx,
                util.keys[util.genesis().validators.quorum_threshold() as usize - 1]
                    .sign_msg(replica_timeout.clone()),
            )
            .await
            .unwrap()
            .unwrap()
            .msg;
        assert_matches!(res.justification, validator::ProposalJustification::Timeout(qc) => {
            assert_eq!(qc.view, replica_timeout.view);
        });
        for i in util.genesis().validators.quorum_threshold() as usize..util.keys.len() {
            let res = util
                .process_replica_timeout(ctx, util.keys[i].sign_msg(replica_timeout.clone()))
                .await;
            assert_matches!(res, Err(timeout::Error::Old { .. }));
        }

        Ok(())
    })
    .await
    .unwrap();
}

/// Check all ReplicaTimeout are included for weight calculation
/// even on different messages for the same view.
#[tokio::test]
async fn timeout_weight_different_messages() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let view = util.view();
        util.produce_block(ctx).await;

        let replica_timeout = util.new_replica_timeout(ctx).await;
        util.replica.phase = validator::Phase::Prepare; // To allow processing of proposal later.
        let proposal = replica_timeout.clone().high_vote.unwrap().proposal;

        // Create a different proposal for the same view
        let mut different_proposal = proposal;
        different_proposal.number = different_proposal.number.next();

        // Create a new ReplicaTimeout with the different proposal
        let mut other_replica_timeout = replica_timeout.clone();
        let mut high_vote = other_replica_timeout.high_vote.clone().unwrap();
        high_vote.proposal = different_proposal;
        let high_qc = util
            .new_commit_qc(ctx, |msg: &mut validator::ReplicaCommit| {
                msg.proposal = different_proposal;
                msg.view = view;
            })
            .await;
        other_replica_timeout.high_vote = Some(high_vote);
        other_replica_timeout.high_qc = Some(high_qc);

        let validators = util.keys.len();

        // half of the validators sign replica_timeout
        for i in 0..validators / 2 {
            util.process_replica_timeout(ctx, util.keys[i].sign_msg(replica_timeout.clone()))
                .await
                .unwrap();
        }

        let mut res = None;
        // The rest of the validators until threshold sign other_replica_timeout
        for i in validators / 2..util.genesis().validators.quorum_threshold() as usize {
            res = util
                .process_replica_timeout(ctx, util.keys[i].sign_msg(other_replica_timeout.clone()))
                .await
                .unwrap();
        }

        assert_matches!(res.unwrap().msg.justification, validator::ProposalJustification::Timeout(qc) => {
            assert_eq!(qc.view, replica_timeout.view);
            assert_eq!(qc.high_vote(util.genesis()).unwrap(), proposal);
        });

        Ok(())
    })
    .await
    .unwrap();
}

/// Check that leader won't accumulate undefined amount of messages if
/// it's spammed with ReplicaTimeout messages for future views
#[tokio::test]
async fn replica_timeout_limit_messages_in_memory() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let mut replica_timeout = util.new_replica_timeout(ctx).await;
        let mut view = util.view();
        // Spam it with 200 messages for different views
        for _ in 0..200 {
            replica_timeout.view = view;
            let res = util
                .process_replica_timeout(ctx, util.owner_key().sign_msg(replica_timeout.clone()))
                .await;
            assert_matches!(res, Ok(_));
            view.number = view.number.next();
        }

        // Ensure only 1 timeout_qc is in memory, as the previous 199 were discarded each time
        // a new message was processed
        assert_eq!(util.replica.timeout_qcs_cache.len(), 1);

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replica_timeout_filter_functions_test() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new(ctx, 2).await;
        s.spawn_bg(runner.run(ctx));

        let replica_timeout = util.new_replica_timeout(ctx).await;
        let msg = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaTimeout(
                replica_timeout.clone(),
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
        let mut replica_timeout_from_view_2 = replica_timeout.clone();
        replica_timeout_from_view_2.view.number = validator::ViewNumber(2);
        let msg_from_view_2 = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaTimeout(
                replica_timeout_from_view_2,
            ));
        util.send(msg_from_view_2);

        // Send a msg with view number = 4, will prune message from view 2
        let mut replica_timeout_from_view_4 = replica_timeout.clone();
        replica_timeout_from_view_4.view.number = validator::ViewNumber(4);
        let msg_from_view_4 = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaTimeout(
                replica_timeout_from_view_4,
            ));
        util.send(msg_from_view_4.clone());

        // Send a msg with view number = 3, will be discarded, as it is older than message from view 4
        let mut replica_timeout_from_view_3 = replica_timeout.clone();
        replica_timeout_from_view_3.view.number = validator::ViewNumber(3);
        let msg_from_view_3 = util
            .owner_key()
            .sign_msg(validator::ConsensusMsg::ReplicaTimeout(
                replica_timeout_from_view_3,
            ));
        util.send(msg_from_view_3);

        // Validate only message from view 4 is received
        assert_eq!(
            util.replica.inbound_channel.recv(ctx).await.unwrap().msg,
            msg_from_view_4
        );

        // Send a msg from validator 0
        let msg_from_validator_0 = util.keys[0].sign_msg(validator::ConsensusMsg::ReplicaTimeout(
            replica_timeout.clone(),
        ));
        util.send(msg_from_validator_0.clone());

        // Send a msg from validator 1
        let msg_from_validator_1 = util.keys[1].sign_msg(validator::ConsensusMsg::ReplicaTimeout(
            replica_timeout.clone(),
        ));
        util.send(msg_from_validator_1.clone());

        // Validate both are present in the inbound_channel
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
