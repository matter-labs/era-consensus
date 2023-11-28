//! ## Tests coverage
//!
//! - [x] replica_prepare_sanity
//! - [x] replica_prepare_sanity_yield_leader_prepare
//! - [x] replica_prepare_old_view
//! - [x] replica_prepare_during_commit
//! - [x] replica_prepare_not_leader_in_view
//! - [x] replica_prepare_already_exists
//! - [x] replica_prepare_num_received_below_threshold
//! - [x] replica_prepare_invalid_sig
//! - [x] replica_prepare_invalid_commit_qc
//! - [x] replica_prepare_high_qc_of_current_view
//! - [x] replica_prepare_high_qc_of_future_view
//! - [x] replica_prepare_non_validator_signer // FAILS
//!
//! - [x] replica_commit_sanity
//! - [x] replica_commit_sanity_yield_leader_commit
//! - [x] replica_commit_old
//! - [x] replica_commit_not_leader_in_view
//! - [x] replica_commit_already_exists
//! - [x] replica_commit_num_received_below_threshold
//! - [x] replica_commit_invalid_sig
//! - [x] replica_commit_unexpected_proposal
//! - [x] replica_commit_protocol_version_mismatch // FAILS
//!

use super::{
    replica_commit::Error as ReplicaCommitError, replica_prepare::Error as ReplicaPrepareError,
};
use crate::testonly::ut_harness::UTHarness;
use assert_matches::assert_matches;
use rand::Rng;
use zksync_consensus_roles::validator::{
    self, CommitQC, ConsensusMsg, LeaderCommit, LeaderPrepare, Phase, ProtocolVersion,
    ReplicaCommit, ViewNumber,
};

#[tokio::test]
async fn replica_prepare_sanity() {
    let mut util = UTHarness::new().await;

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    util.dispatch_replica_prepare(replica_prepare).unwrap();
}

#[tokio::test]
async fn replica_prepare_sanity_yield_leader_prepare() {
    let mut util = UTHarness::new().await;

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    util.dispatch_replica_prepare(replica_prepare).unwrap();
    util.recv_signed()
        .await
        .unwrap()
        .cast::<LeaderPrepare>()
        .unwrap();
}

#[tokio::test]
async fn replica_prepare_old_view() {
    let mut util = UTHarness::new().await;

    util.set_replica_view(ViewNumber(1));
    util.set_leader_view(ViewNumber(2));
    util.set_leader_phase(Phase::Prepare);

    let replica_prepare = util.new_current_replica_prepare(|_| {});
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

    let replica_prepare = util.new_current_replica_prepare(|_| {});
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
async fn replica_prepare_not_leader_in_view() {
    let mut util = UTHarness::new_with(2).await;

    let current_view_leader = util.view_leader(util.current_replica_view());
    assert_ne!(current_view_leader, util.own_key().public());

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare(replica_prepare.clone());
    assert_matches!(res, Err(ReplicaPrepareError::NotLeaderInView));
}

#[tokio::test]
async fn replica_prepare_already_exists() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.own_key().public());

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare(replica_prepare.clone());
    let res = util.dispatch_replica_prepare(replica_prepare.clone());
    assert_matches!(
    res,
    Err(ReplicaPrepareError::Exists { existing_message }) => {
            assert_eq!(existing_message, replica_prepare.cast().unwrap().msg);
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

    let replica_prepare = util.new_current_replica_prepare(|_| {});
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
async fn replica_prepare_invalid_sig() {
    let mut util = UTHarness::new().await;

    let mut replica_prepare = util.new_current_replica_prepare(|_| {});
    replica_prepare.sig = util.rng().gen();
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(res, Err(ReplicaPrepareError::InvalidSignature(_)));
}

#[tokio::test]
async fn replica_prepare_invalid_commit_qc() {
    let mut util = UTHarness::new().await;

    let junk = util.rng().gen::<CommitQC>();
    let replica_prepare = util.new_current_replica_prepare(|msg| msg.high_qc = junk);
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(res, Err(ReplicaPrepareError::InvalidHighQC(..)));
}

#[tokio::test]
async fn replica_prepare_high_qc_of_current_view() {
    let mut util = UTHarness::new().await;

    let view = ViewNumber(1);
    let qc_view = ViewNumber(1);

    util.set_view(view);
    let qc = util.new_commit_qc(|msg| {
        msg.view = qc_view;
    });
    let replica_prepare = util.new_current_replica_prepare(|msg| msg.high_qc = qc);
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(
        res,
        Err(ReplicaPrepareError::HighQCOfFutureView { high_qc_view, current_view }) => {
            assert_eq!(high_qc_view, qc_view);
            assert_eq!(current_view, view);
        }
    );
}

#[tokio::test]
async fn replica_prepare_high_qc_of_future_view() {
    let mut util = UTHarness::new().await;

    let view = ViewNumber(1);
    let qc_view = ViewNumber(2);

    util.set_view(view);
    let qc = util.new_commit_qc(|msg| {
        msg.view = qc_view;
    });

    let replica_prepare = util.new_current_replica_prepare(|msg| msg.high_qc = qc);
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(
        res,
        Err(ReplicaPrepareError::HighQCOfFutureView{ high_qc_view, current_view }) => {
            assert_eq!(high_qc_view, qc_view);
            assert_eq!(current_view, view);
        }
    );
}

#[ignore = "fails/unsupported"]
#[tokio::test]
async fn replica_prepare_non_validator_signer() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_view(view);
    assert_eq!(util.view_leader(view), util.key_at(0).public());

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare(replica_prepare.clone());

    let non_validator: validator::SecretKey = util.rng().gen();
    let replica_prepare = non_validator.sign_msg(replica_prepare.msg);
    util.dispatch_replica_prepare(replica_prepare).unwrap();
    // PANICS:
    // "Couldn't create justification from valid replica messages!: Message signer isn't in the validator set"
}

#[tokio::test]
async fn replica_commit_sanity() {
    let mut util = UTHarness::new().await;

    let leader_prepare = util.new_procedural_replica_commit().await;
    util.dispatch_replica_commit(leader_prepare).unwrap();
}

#[tokio::test]
async fn replica_commit_sanity_yield_leader_commit() {
    let mut util = UTHarness::new().await;

    let replica_commit = util.new_procedural_replica_commit().await;
    util.dispatch_replica_commit(replica_commit).unwrap();
    util.recv_signed()
        .await
        .unwrap()
        .cast::<LeaderCommit>()
        .unwrap();
}

#[tokio::test]
async fn replica_commit_old() {
    let mut util = UTHarness::new().await;

    let mut replica_commit = util
        .new_procedural_replica_commit()
        .await
        .cast::<ReplicaCommit>()
        .unwrap()
        .msg;
    replica_commit.view = util.current_replica_view().prev();
    let replica_commit = util
        .own_key()
        .sign_msg(ConsensusMsg::ReplicaCommit(replica_commit));

    let res = util.dispatch_replica_commit(replica_commit);
    assert_matches!(
    res,
    Err(ReplicaCommitError::Old { current_view, current_phase }) => {
        assert_eq!(current_view, util.current_replica_view());
        assert_eq!(current_phase, util.current_replica_phase());
    });
}

#[tokio::test]
async fn replica_commit_not_leader_in_view() {
    let mut util = UTHarness::new_with(2).await;

    let current_view_leader = util.view_leader(util.current_replica_view());
    assert_ne!(current_view_leader, util.own_key().public());

    let replica_commit = util.new_current_replica_commit(|_| {});
    let res = util.dispatch_replica_commit(replica_commit);
    assert_matches!(res, Err(ReplicaCommitError::NotLeaderInView));
}

#[tokio::test]
async fn replica_commit_already_exists() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.own_key().public());

    let replica_prepare_one = util.new_current_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare(replica_prepare_one.clone());
    let replica_prepare_two = util.key_at(1).sign_msg(replica_prepare_one.msg);
    util.dispatch_replica_prepare(replica_prepare_two).unwrap();

    let leader_prepare = util.recv_signed().await.unwrap();
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();

    let replica_commit = util.recv_signed().await.unwrap();
    let _ = util.dispatch_replica_commit(replica_commit.clone());
    let res = util.dispatch_replica_commit(replica_commit.clone());
    assert_matches!(
        res,
        Err(ReplicaCommitError::DuplicateMessage { existing_message }) => {
            assert_eq!(existing_message, replica_commit.cast::<ReplicaCommit>().unwrap().msg)
        }
    );
}

#[tokio::test]
async fn replica_commit_num_received_below_threshold() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.own_key().public());

    let replica_prepare_one = util.new_current_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare(replica_prepare_one.clone());
    let replica_prepare_two = util.key_at(1).sign_msg(replica_prepare_one.msg);
    util.dispatch_replica_prepare(replica_prepare_two).unwrap();

    let leader_prepare = util.recv_signed().await.unwrap();
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();

    let replica_commit = util.recv_signed().await.unwrap();
    let res = util.dispatch_replica_commit(replica_commit.clone());
    assert_matches!(
        res,
        Err(ReplicaCommitError::NumReceivedBelowThreshold {
            num_messages: 1,
            threshold: 2
        })
    );
}

#[tokio::test]
async fn replica_commit_invalid_sig() {
    let mut util = UTHarness::new().await;

    let mut replica_commit = util.new_current_replica_commit(|_| {});
    replica_commit.sig = util.rng().gen();
    let res = util.dispatch_replica_commit(replica_commit);
    assert_matches!(res, Err(ReplicaCommitError::InvalidSignature(..)));
}

#[tokio::test]
async fn replica_commit_unexpected_proposal() {
    let mut util = UTHarness::new().await;

    let replica_commit = util.new_current_replica_commit(|_| {});
    let res = util.dispatch_replica_commit(replica_commit);
    assert_matches!(res, Err(ReplicaCommitError::UnexpectedProposal));
}

#[ignore = "fails/unsupported"]
#[tokio::test]
async fn replica_commit_protocol_version_mismatch() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.own_key().public());

    let replica_prepare_one = util.new_current_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare(replica_prepare_one.clone());
    let replica_prepare_two = util.key_at(1).sign_msg(replica_prepare_one.msg);
    util.dispatch_replica_prepare(replica_prepare_two).unwrap();

    let leader_prepare = util.recv_signed().await.unwrap();
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();

    let replica_commit = util.recv_signed().await.unwrap();
    let _ = util.dispatch_replica_commit(replica_commit.clone());

    let mut replica_commit_two = replica_commit.cast::<ReplicaCommit>().unwrap().msg;
    replica_commit_two.protocol_version =
        ProtocolVersion(replica_commit_two.protocol_version.0 + 1);

    let replica_commit_two = util
        .key_at(1)
        .sign_msg(ConsensusMsg::ReplicaCommit(replica_commit_two));
    util.dispatch_replica_commit(replica_commit_two).unwrap();
    // PANICS:
    // "Couldn't create justification from valid replica messages!: CommitQC can only be created from votes for the same message."
}
