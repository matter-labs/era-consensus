use super::{
    replica_commit::Error as ReplicaCommitError, replica_prepare::Error as ReplicaPrepareError,
};
use crate::testonly::ut_harness::UTHarness;
use assert_matches::assert_matches;
use rand::Rng;
use zksync_consensus_roles::validator::{
    self, CommitQC, ConsensusMsg, LeaderCommit, LeaderPrepare, Phase, PrepareQC, ProtocolVersion,
    ReplicaCommit, ReplicaPrepare, ViewNumber,
};

#[tokio::test]
async fn replica_prepare_sanity() {
    let mut util = UTHarness::new_many().await;

    util.set_view(util.owner_as_view_leader());

    let replica_prepare = util.new_current_replica_prepare(|_| {}).cast().unwrap().msg;
    util.dispatch_replica_prepare_many(
        vec![replica_prepare; util.consensus_threshold()],
        util.keys(),
    )
    .unwrap();
}

#[tokio::test]
async fn replica_prepare_sanity_yield_leader_prepare() {
    let mut util = UTHarness::new_one().await;

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    util.dispatch_replica_prepare_one(replica_prepare.clone())
        .unwrap();
    let leader_prepare = util
        .recv_signed()
        .await
        .unwrap()
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;

    let replica_prepare = replica_prepare.cast::<ReplicaPrepare>().unwrap().msg;
    assert_matches!(
        leader_prepare,
        LeaderPrepare {
            protocol_version,
            view,
            proposal,
            proposal_payload: _,
            justification,
        } => {
            assert_eq!(protocol_version, replica_prepare.protocol_version);
            assert_eq!(view, replica_prepare.view);
            assert_eq!(proposal.parent, replica_prepare.high_vote.proposal.hash());
            assert_eq!(justification, util.new_prepare_qc(|msg| *msg = replica_prepare));
        }
    );
}

#[tokio::test]
async fn replica_prepare_sanity_yield_leader_prepare_reproposal() {
    let mut util = UTHarness::new_many().await;

    util.set_view(util.owner_as_view_leader());

    let replica_prepare: ReplicaPrepare =
        util.new_unfinalized_replica_prepare().cast().unwrap().msg;
    util.dispatch_replica_prepare_many(
        vec![replica_prepare.clone(); util.consensus_threshold()],
        util.keys(),
    )
    .unwrap();
    let leader_prepare = util
        .recv_signed()
        .await
        .unwrap()
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;

    assert_matches!(
        leader_prepare,
        LeaderPrepare {
            protocol_version,
            view,
            proposal,
            proposal_payload,
            justification,
        } => {
            assert_eq!(protocol_version, replica_prepare.protocol_version);
            assert_eq!(view, replica_prepare.view);
            assert_eq!(proposal, replica_prepare.high_vote.proposal);
            assert_eq!(proposal_payload, None);
            assert_matches!(
                justification,
                PrepareQC { map, .. } => {
                    assert_eq!(map.len(), 1);
                    assert_eq!(*map.first_key_value().unwrap().0, replica_prepare);
                }
            );
        }
    );
}

#[tokio::test]
async fn replica_prepare_old_view() {
    let mut util = UTHarness::new_one().await;

    util.set_replica_view(ViewNumber(1));
    util.set_leader_view(ViewNumber(2));
    util.set_leader_phase(Phase::Prepare);

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare_one(replica_prepare);
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
    let mut util = UTHarness::new_one().await;

    util.set_leader_phase(Phase::Commit);

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare_one(replica_prepare);
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
    assert_ne!(current_view_leader, util.owner_key().public());

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare_one(replica_prepare.clone());
    assert_matches!(res, Err(ReplicaPrepareError::NotLeaderInView));
}

#[tokio::test]
async fn replica_prepare_already_exists() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.owner_key().public());

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare_one(replica_prepare.clone());
    let res = util.dispatch_replica_prepare_one(replica_prepare.clone());
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
    assert_eq!(util.view_leader(view), util.owner_key().public());

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare_one(replica_prepare);
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
    let mut util = UTHarness::new_one().await;

    let mut replica_prepare = util.new_current_replica_prepare(|_| {});
    replica_prepare.sig = util.rng().gen();
    let res = util.dispatch_replica_prepare_one(replica_prepare);
    assert_matches!(res, Err(ReplicaPrepareError::InvalidSignature(_)));
}

#[tokio::test]
async fn replica_prepare_invalid_commit_qc() {
    let mut util = UTHarness::new_one().await;

    let junk = util.rng().gen::<CommitQC>();
    let replica_prepare = util.new_current_replica_prepare(|msg| msg.high_qc = junk);
    let res = util.dispatch_replica_prepare_one(replica_prepare);
    assert_matches!(res, Err(ReplicaPrepareError::InvalidHighQC(..)));
}

#[tokio::test]
async fn replica_prepare_high_qc_of_current_view() {
    let mut util = UTHarness::new_one().await;

    let view = ViewNumber(1);
    let qc_view = ViewNumber(1);

    util.set_view(view);
    let qc = util.new_commit_qc(|msg| {
        msg.view = qc_view;
    });
    let replica_prepare = util.new_current_replica_prepare(|msg| msg.high_qc = qc);
    let res = util.dispatch_replica_prepare_one(replica_prepare);
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
    let mut util = UTHarness::new_one().await;

    let view = ViewNumber(1);
    let qc_view = ViewNumber(2);

    util.set_view(view);
    let qc = util.new_commit_qc(|msg| {
        msg.view = qc_view;
    });

    let replica_prepare = util.new_current_replica_prepare(|msg| msg.high_qc = qc);
    let res = util.dispatch_replica_prepare_one(replica_prepare);
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
    let _ = util.dispatch_replica_prepare_one(replica_prepare.clone());

    let non_validator: validator::SecretKey = util.rng().gen();
    let replica_prepare = non_validator.sign_msg(replica_prepare.msg);
    util.dispatch_replica_prepare_one(replica_prepare).unwrap();
    // PANICS:
    // "Couldn't create justification from valid replica messages!: Message signer isn't in the validator set"
}

#[tokio::test]
async fn replica_commit_sanity() {
    let mut util = UTHarness::new_many().await;

    util.set_view(util.owner_as_view_leader());

    let replica_commit = util
        .new_procedural_replica_commit_many()
        .await
        .cast()
        .unwrap()
        .msg;
    util.dispatch_replica_commit_many(
        vec![replica_commit; util.consensus_threshold()],
        util.keys(),
    )
    .unwrap();
}

#[tokio::test]
async fn replica_commit_sanity_yield_leader_commit() {
    let mut util = UTHarness::new_one().await;

    let replica_commit = util.new_procedural_replica_commit_one().await;
    util.dispatch_replica_commit_one(replica_commit.clone())
        .unwrap();
    let leader_commit = util
        .recv_signed()
        .await
        .unwrap()
        .cast::<LeaderCommit>()
        .unwrap()
        .msg;

    let replica_commit = replica_commit.cast::<ReplicaCommit>().unwrap().msg;
    assert_matches!(
        leader_commit,
        LeaderCommit {
            protocol_version,
            justification,
        } => {
            assert_eq!(protocol_version, replica_commit.protocol_version);
            assert_eq!(justification, util.new_commit_qc(|msg| *msg = replica_commit));
        }
    );
}

#[tokio::test]
async fn replica_commit_old() {
    let mut util = UTHarness::new_one().await;

    let mut replica_commit = util
        .new_procedural_replica_commit_one()
        .await
        .cast::<ReplicaCommit>()
        .unwrap()
        .msg;
    replica_commit.view = util.current_replica_view().prev();
    let replica_commit = util
        .owner_key()
        .sign_msg(ConsensusMsg::ReplicaCommit(replica_commit));

    let res = util.dispatch_replica_commit_one(replica_commit);
    assert_matches!(
        res,
        Err(ReplicaCommitError::Old { current_view, current_phase }) => {
            assert_eq!(current_view, util.current_replica_view());
            assert_eq!(current_phase, util.current_replica_phase());
        }
    );
}

#[tokio::test]
async fn replica_commit_not_leader_in_view() {
    let mut util = UTHarness::new_with(2).await;

    let current_view_leader = util.view_leader(util.current_replica_view());
    assert_ne!(current_view_leader, util.owner_key().public());

    let replica_commit = util.new_current_replica_commit(|_| {});
    let res = util.dispatch_replica_commit_one(replica_commit);
    assert_matches!(res, Err(ReplicaCommitError::NotLeaderInView));
}

#[tokio::test]
async fn replica_commit_already_exists() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.owner_key().public());

    let replica_prepare_one = util.new_current_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare_one(replica_prepare_one.clone());
    let replica_prepare_two = util.key_at(1).sign_msg(replica_prepare_one.msg);
    util.dispatch_replica_prepare_one(replica_prepare_two)
        .unwrap();

    let leader_prepare = util.recv_signed().await.unwrap();
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();

    let replica_commit = util.recv_signed().await.unwrap();
    let _ = util.dispatch_replica_commit_one(replica_commit.clone());
    let res = util.dispatch_replica_commit_one(replica_commit.clone());
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
    assert_eq!(util.view_leader(view), util.owner_key().public());

    let replica_prepare_one = util.new_current_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare_one(replica_prepare_one.clone());
    let replica_prepare_two = util.key_at(1).sign_msg(replica_prepare_one.msg);
    util.dispatch_replica_prepare_one(replica_prepare_two)
        .unwrap();

    let leader_prepare = util.recv_signed().await.unwrap();
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();

    let replica_commit = util.recv_signed().await.unwrap();
    let res = util.dispatch_replica_commit_one(replica_commit.clone());
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
    let mut util = UTHarness::new_one().await;

    let mut replica_commit = util.new_current_replica_commit(|_| {});
    replica_commit.sig = util.rng().gen();
    let res = util.dispatch_replica_commit_one(replica_commit);
    assert_matches!(res, Err(ReplicaCommitError::InvalidSignature(..)));
}

#[tokio::test]
async fn replica_commit_unexpected_proposal() {
    let mut util = UTHarness::new_one().await;

    let replica_commit = util.new_current_replica_commit(|_| {});
    let res = util.dispatch_replica_commit_one(replica_commit);
    assert_matches!(res, Err(ReplicaCommitError::UnexpectedProposal));
}

#[ignore = "fails/unsupported"]
#[tokio::test]
async fn replica_commit_protocol_version_mismatch() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.owner_key().public());

    let replica_prepare_one = util.new_current_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare_one(replica_prepare_one.clone());
    let replica_prepare_two = util.key_at(1).sign_msg(replica_prepare_one.msg);
    util.dispatch_replica_prepare_one(replica_prepare_two)
        .unwrap();

    let leader_prepare = util.recv_signed().await.unwrap();
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();

    let replica_commit = util.recv_signed().await.unwrap();
    let _ = util.dispatch_replica_commit_one(replica_commit.clone());

    let mut replica_commit_two = replica_commit.cast::<ReplicaCommit>().unwrap().msg;
    replica_commit_two.protocol_version =
        ProtocolVersion(replica_commit_two.protocol_version.0 + 1);

    let replica_commit_two = util
        .key_at(1)
        .sign_msg(ConsensusMsg::ReplicaCommit(replica_commit_two));
    util.dispatch_replica_commit_one(replica_commit_two)
        .unwrap();
    // PANICS:
    // "Couldn't create justification from valid replica messages!: CommitQC can only be created from votes for the same message."
}
