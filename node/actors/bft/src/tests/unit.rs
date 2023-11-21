use crate::{
    leader::ReplicaPrepareError, replica::LeaderPrepareError, testonly::ut_harness::UTHarness,
};
use assert_matches::assert_matches;
use rand::Rng;
use zksync_consensus_crypto::bn254::Error::SignatureVerificationFailure;
use zksync_consensus_roles::{validator::{ConsensusMsg, Phase, ViewNumber},
};

/// ## Tests coverage
///
/// - [x] replica_prepare_sanity
/// - [ ] replica_prepare_sanity_yield_leader_prepare
/// - [x] replica_prepare_old_view
/// - [x] replica_prepare_during_commit
/// - [ ] replica_prepare_not_leader_in_view
/// - [ ] replica_prepare_already_exists
/// - [ ] replica_prepare_invalid_sig
/// - [ ] replica_prepare_invalid_commit_qc_different_views
/// - [ ] replica_prepare_invalid_commit_qc_signers_list_empty
/// - [ ] replica_prepare_invalid_commit_qc_signers_list_invalid_size
/// - [ ] replica_prepare_invalid_commit_qc_multi_msg_per_signer
/// - [ ] replica_prepare_invalid_commit_qc_insufficient_signers
/// - [ ] replica_prepare_invalid_commit_qc_invalid_aggregate_sig
/// - [ ] replica_prepare_high_qc_of_future_view
/// - [x] replica_prepare_num_received_below_threshold
/// -
/// - [x] leader_prepare_sanity
/// - [ ] leader_prepare_sanity_yield_replica_commit
/// - [x] leader_prepare_invalid_leader
/// - [x] leader_prepare_old_view
/// - [x] leader_prepare_invalid_sig
/// - [x] leader_prepare_invalid_prepare_qc_different_views
/// - [ ] leader_prepare_invalid_prepare_qc_signers_list_empty
/// - [ ] leader_prepare_invalid_prepare_qc_signers_list_invalid_size
/// - [ ] leader_prepare_invalid_prepare_qc_multi_msg_per_signer
/// - [ ] leader_prepare_invalid_prepare_qc_insufficient_signers
/// - [ ] leader_prepare_invalid_prepare_qc_invalid_aggregate_sig
/// - [ ] leader_prepare_high_qc_of_future_view
/// - [ ] leader_prepare_proposal_oversized_payload
/// - [ ] leader_prepare_proposal_mismatched_payload
/// - [ ] leader_prepare_proposal_when_previous_not_finalized
/// - [ ] leader_prepare_proposal_invalid_parent_hash
/// - [ ] leader_prepare_proposal_non_sequential_number
/// - [ ] leader_prepare_reproposal_without_quorum
/// - [ ] leader_prepare_reproposal_when_finalized
/// - [ ] leader_prepare_reproposal_invalid_block
/// - [ ] leader_prepare_yield_replica_commit
/// -
/// - [ ] replica_commit_sanity
/// - [ ] replica_commit_sanity_yield_leader_commit
/// - [ ] replica_commit_old
/// - [ ] replica_commit_not_leader_in_view
/// - [ ] replica_commit_already_exists
/// - [ ] replica_commit_invalid_sig
/// - [ ] replica_commit_unexpected_proposal
/// - [ ] replica_commit_num_received_below_threshold
/// - [ ] replica_commit_make_commit_qc_failure_distinct_messages
/// -
/// - [ ] leader_commit_sanity
/// - [ ] leader_commit_sanity_yield_replica_prepare
/// - [ ] leader_commit_invalid_leader
/// - [ ] leader_commit_old
/// - [ ] leader_commit_invalid_sig
/// - [ ] leader_commit_invalid_commit_qc_signers_list_empty
/// - [ ] leader_commit_invalid_commit_qc_signers_list_invalid_size
/// - [ ] leader_commit_invalid_commit_qc_insufficient_signers
/// - [ ] leader_commit_invalid_commit_qc_invalid_aggregate_sig
///
#[tokio::test]
async fn replica_prepare_sanity() {
    let mut util = UTHarness::new().await;

    let replica_prepare = util.new_replica_prepare(|_| {});
    util.dispatch_replica_prepare(replica_prepare).unwrap();
}

#[tokio::test]
async fn replica_prepare_sanity_yield_leader_prepare() {
    let mut util = UTHarness::new().await;

    let replica_prepare = util.new_replica_prepare(|_| {});
    util.dispatch_replica_prepare(replica_prepare).unwrap();
    let _ = util.recv_leader_prepare().await.unwrap();
}

#[tokio::test]
async fn replica_prepare_old_view() {
    let mut util = UTHarness::new().await;

    util.set_replica_view(ViewNumber(1));
    util.set_leader_view(ViewNumber(2));
    util.set_leader_phase(Phase::Prepare);

    let replica_prepare = util.new_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare(replica_prepare);

    assert_matches!(
        res,
        Err(ReplicaPrepareError::Old {
            current_view: ViewNumber(2),
            current_phase: Phase::Prepare,
        })
    );
}

#[tokio::test]
async fn replica_prepare_during_commit() {
    let mut util = UTHarness::new().await;

    util.set_leader_phase(Phase::Commit);

    let replica_prepare = util.new_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare(replica_prepare);

    assert_matches!(
        res,
        Err(ReplicaPrepareError::Old {
            current_view: ViewNumber(1),
            current_phase: Phase::Commit,
        })
    );
}

#[tokio::test]
async fn replica_prepare_already_exists() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);

    assert_eq!(util.view_leader(view), util.own_key().public());
    let replica_prepare = util.new_replica_prepare(|_| {});

    let res = util.dispatch_replica_prepare(replica_prepare.clone());
    assert_matches!(
        res,
        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
            num_messages: 1,
            threshold: 2
        })
    );

    let res = util.dispatch_replica_prepare(replica_prepare.clone());
    assert_matches!(
    res,
    Err(ReplicaPrepareError::Exists {existing_message: msg }) => {
            assert_eq!(msg, replica_prepare.cast().unwrap().msg);
        }
    );
}

#[tokio::test]
async fn replica_prepare_num_received_below_threshold() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.own_key().public());

    let replica_prepare = util.new_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(
        res,
        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
            num_messages: 1,
            threshold: 2
        })
    );
}

#[tokio::test]
async fn leader_prepare_sanity() {
    let mut util = UTHarness::new().await;

    let replica_prepare = util.new_replica_prepare(|_| {});
    util.dispatch_replica_prepare(replica_prepare).unwrap();
    let leader_prepare = util.recv_signed().await.unwrap();
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();
}

#[tokio::test]
async fn leader_prepare_invalid_leader() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);

    assert_eq!(util.view_leader(view), util.key_at(0).public());

    let replica_prepare_one = util.new_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare(replica_prepare_one.clone());
    assert_matches!(
        res,
        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
            num_messages: 1,
            threshold: 2,
        })
    );

    let replica_prepare_two = util.key_at(1).sign_msg(replica_prepare_one.msg);
    util.dispatch_replica_prepare(replica_prepare_two).unwrap();

    let mut leader_prepare = util.recv_leader_prepare().await.unwrap();
    leader_prepare.view = leader_prepare.view.next();
    assert_ne!(
        util.view_leader(leader_prepare.view),
        util.key_at(0).public()
    );
    let leader_prepare = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));
    let res = util.dispatch_leader_prepare(leader_prepare).await;

    assert_matches!(
        res,
        Err(LeaderPrepareError::InvalidLeader { correct_leader, received_leader }) => {
            assert_eq!(correct_leader, util.key_at(1).public());
            assert_eq!(received_leader, util.key_at(0).public());
        }
    );
}

#[tokio::test]
async fn leader_prepare_invalid_sig() {
    let mut util = UTHarness::new().await;

    let mut leader_prepare = util.new_leader_prepare(|_| {});
    leader_prepare.sig = util.rng().gen();
    let res = util.dispatch_leader_prepare(leader_prepare).await;

    assert_matches!(
        res,
        Err(LeaderPrepareError::InvalidSignature(
            SignatureVerificationFailure
        ))
    );
}

#[tokio::test]
async fn leader_prepare_invalid_prepare_qc_different_views() {
    let mut util = UTHarness::new().await;

    let replica_prepare = util.new_replica_prepare(|_| {});
    util.dispatch_replica_prepare(replica_prepare.clone()).unwrap();

    let mut leader_prepare = util.recv_leader_prepare().await.unwrap();
    leader_prepare.view = leader_prepare.view.next();
    let leader_prepare = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));
    let res = util.dispatch_leader_prepare(leader_prepare).await;

    assert_matches!(
    res,
    Err(LeaderPrepareError::InvalidPrepareQC(err)) => {
        assert_eq!(err.to_string(), "PrepareQC contains messages for different views!")
        }
    )
}

// #[tokio::test]
// async fn replica_commit_sanity() {
//     let mut util = Util::make().await;
//
//     let replica_prepare = util::make_replica_prepare(&consensus, None::<fn(&mut ReplicaPrepare)>);
//     let res = util::dispatch_replica_prepare(&ctx, &mut consensus, replica_prepare);
//     assert_matches!(res, Ok(()));
//     let leader_prepare = util::make_leader_prepare_from_replica_prepare(&consensus, &mut rng, replica_prepare, Some(|msg: &mut LeaderPrepare| {
//         msg.view = ViewNumber(2);
//     }));
//     let replica_commit = util::make_replica_commit(&consensus, leader_prepare.msg., None::<fn(&mut ReplicaCommit)>);
//
//     let res = scope::run!(&ctx, |ctx, s| {
//         s.spawn_blocking(|| {
//             let res = consensus
//             .leader
//             .process_replica_commit(ctx, &consensus.inner, leader_prepare.cast().unwrap());
//             Ok(res)
//         })
//         .join(ctx)
//     })
//         .await
//         .unwrap();
//
//     assert_matches!(
//         res,
//         Err(LeaderPrepareError::InvalidPrepareQC(anyhow!("PrepareQC contains messages for different views!")))
//     );
// }
