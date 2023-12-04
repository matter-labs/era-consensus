use super::{
    leader_commit::Error as LeaderCommitError, leader_prepare::Error as LeaderPrepareError,
};
use crate::{inner::ConsensusInner, leader::ReplicaPrepareError, testonly::ut_harness::UTHarness};
use assert_matches::assert_matches;
use rand::Rng;
use std::cell::RefCell;
use zksync_consensus_roles::validator::{
    BlockHeaderHash, ConsensusMsg, LeaderCommit, LeaderPrepare, Payload, PrepareQC, ReplicaCommit,
    ReplicaPrepare, ViewNumber,
};

#[tokio::test]
async fn leader_prepare_sanity() {
    let mut util = UTHarness::new_many().await;

    util.set_view(util.owner_as_view_leader());

    let leader_prepare = util.new_procedural_leader_prepare_many().await;
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();
}

#[tokio::test]
async fn leader_prepare_reproposal_sanity() {
    let mut util = UTHarness::new_many().await;

    util.set_view(util.owner_as_view_leader());

    let replica_prepare: ReplicaPrepare =
        util.new_unfinalized_replica_prepare().cast().unwrap().msg;
    util.dispatch_replica_prepare_many(
        vec![replica_prepare.clone(); util.consensus_threshold()],
        util.keys(),
    )
    .unwrap();
    let leader_prepare_signed = util.recv_signed().await.unwrap();

    let leader_prepare = leader_prepare_signed
        .clone()
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    assert_matches!(
        leader_prepare,
        LeaderPrepare {proposal_payload, .. } => {
            assert_eq!(proposal_payload, None);
        }
    );

    util.dispatch_leader_prepare(leader_prepare_signed)
        .await
        .unwrap();
}

#[tokio::test]
async fn leader_prepare_incompatible_protocol_version() {
    let mut util = UTHarness::new_one().await;

    let incompatible_protocol_version = util.incompatible_protocol_version();
    let leader_prepare = util.new_rnd_leader_prepare(|msg| {
        msg.protocol_version = incompatible_protocol_version;
    });
    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::IncompatibleProtocolVersion { message_version, local_version }) => {
            assert_eq!(message_version, incompatible_protocol_version);
            assert_eq!(local_version, util.protocol_version());
        }
    )
}

#[tokio::test]
async fn leader_prepare_sanity_yield_replica_commit() {
    let mut util = UTHarness::new_one().await;

    let leader_prepare = util.new_procedural_leader_prepare_one().await;
    util.dispatch_leader_prepare(leader_prepare.clone())
        .await
        .unwrap();
    let replica_commit = util
        .recv_signed()
        .await
        .unwrap()
        .cast::<ReplicaCommit>()
        .unwrap()
        .msg;

    let leader_prepare = leader_prepare.cast::<LeaderPrepare>().unwrap().msg;
    assert_matches!(
        replica_commit,
        ReplicaCommit {
            protocol_version,
            view,
            proposal,
        } => {
            assert_eq!(protocol_version, leader_prepare.protocol_version);
            assert_eq!(view, leader_prepare.view);
            assert_eq!(proposal, leader_prepare.proposal);
        }
    );
}

#[tokio::test]
async fn leader_prepare_invalid_leader() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_view(view);
    assert_eq!(util.view_leader(view), util.key_at(0).public());

    let replica_prepare_one = util.new_current_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare_one(replica_prepare_one.clone());
    assert_matches!(
        res,
        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
            num_messages: 1,
            threshold: 2,
        })
    );

    let replica_prepare_two = util.key_at(1).sign_msg(replica_prepare_one.msg);
    util.dispatch_replica_prepare_one(replica_prepare_two)
        .unwrap();
    let msg = util.recv_signed().await.unwrap();
    let mut leader_prepare = msg.cast::<LeaderPrepare>().unwrap().msg;

    leader_prepare.view = leader_prepare.view.next();
    assert_ne!(
        util.view_leader(leader_prepare.view),
        util.key_at(0).public()
    );

    let leader_prepare = util
        .owner_key()
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
async fn leader_prepare_old_view() {
    let mut util = UTHarness::new_one().await;

    let mut leader_prepare = util
        .new_procedural_leader_prepare_one()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.view = util.current_replica_view().prev();
    let leader_prepare = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));

    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::Old { current_view, current_phase }) => {
            assert_eq!(current_view, util.current_replica_view());
            assert_eq!(current_phase, util.current_replica_phase());
        }
    );
}

#[tokio::test]
async fn leader_prepare_invalid_sig() {
    let mut util = UTHarness::new_one().await;

    let mut leader_prepare = util.new_rnd_leader_prepare(|_| {});
    leader_prepare.sig = util.rng().gen();

    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::InvalidSignature(..)));
}

#[tokio::test]
async fn leader_prepare_invalid_prepare_qc() {
    let mut util = UTHarness::new_one().await;

    let mut leader_prepare = util
        .new_procedural_leader_prepare_one()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.justification = util.rng().gen::<PrepareQC>();
    let leader_prepare = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));

    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::InvalidPrepareQC(err)) => {
            assert_eq!(err.to_string(), "PrepareQC contains messages for different views!")
        }
    );
}

#[tokio::test]
async fn leader_prepare_invalid_high_qc() {
    let mut util = UTHarness::new_one().await;

    let mut replica_prepare = util
        .new_current_replica_prepare(|_| {})
        .cast::<ReplicaPrepare>()
        .unwrap()
        .msg;
    replica_prepare.high_qc = util.rng().gen();

    let mut leader_prepare = util
        .new_procedural_leader_prepare_one()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;

    let high_qc = util.rng().gen();
    leader_prepare.justification = util.new_prepare_qc(|msg| msg.high_qc = high_qc);
    let leader_prepare = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));

    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::InvalidHighQC(_)));
}

#[tokio::test]
async fn leader_prepare_proposal_oversized_payload() {
    let mut util = UTHarness::new_one().await;
    let payload_oversize = ConsensusInner::PAYLOAD_MAX_SIZE + 1;
    let payload_vec = vec![0; payload_oversize];

    let mut leader_prepare = util
        .new_procedural_leader_prepare_one()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.proposal_payload = Some(Payload(payload_vec));
    let leader_prepare_signed = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalOversizedPayload{ payload_size, header }) => {
            assert_eq!(payload_size, payload_oversize);
            assert_eq!(header, leader_prepare.proposal);
        }
    );
}

#[tokio::test]
async fn leader_prepare_proposal_mismatched_payload() {
    let mut util = UTHarness::new_one().await;

    let mut leader_prepare = util
        .new_procedural_leader_prepare_one()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.proposal_payload = Some(util.rng().gen());
    let leader_prepare_signed = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(res, Err(LeaderPrepareError::ProposalMismatchedPayload));
}

#[tokio::test]
async fn leader_prepare_proposal_when_previous_not_finalized() {
    let mut util = UTHarness::new_one().await;

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    util.dispatch_replica_prepare_one(replica_prepare.clone())
        .unwrap();

    let mut leader_prepare = util
        .recv_signed()
        .await
        .unwrap()
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;

    let high_vote = util.rng().gen();
    leader_prepare.justification = util.new_prepare_qc(|msg| msg.high_vote = high_vote);

    let leader_prepare_signed = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalWhenPreviousNotFinalized)
    );
}

#[tokio::test]
async fn leader_prepare_proposal_invalid_parent_hash() {
    let mut util = UTHarness::new_one().await;

    let replica_prepare_signed = util.new_current_replica_prepare(|_| {});
    let replica_prepare = replica_prepare_signed
        .clone()
        .cast::<ReplicaPrepare>()
        .unwrap()
        .msg;
    util.dispatch_replica_prepare_one(replica_prepare_signed.clone())
        .unwrap();
    let mut leader_prepare = util
        .recv_signed()
        .await
        .unwrap()
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;

    let junk: BlockHeaderHash = util.rng().gen();
    leader_prepare.proposal.parent = junk;
    let leader_prepare_signed = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalInvalidParentHash {
            correct_parent_hash,
            received_parent_hash,
            header
        }) => {
            assert_eq!(correct_parent_hash, replica_prepare.high_vote.proposal.hash());
            assert_eq!(received_parent_hash, junk);
            assert_eq!(header, leader_prepare.proposal);
        }
    );
}

#[tokio::test]
async fn leader_prepare_proposal_non_sequential_number() {
    let mut util = UTHarness::new_one().await;

    let replica_prepare_signed = util.new_current_replica_prepare(|_| {});
    let replica_prepare = replica_prepare_signed
        .clone()
        .cast::<ReplicaPrepare>()
        .unwrap()
        .msg;
    util.dispatch_replica_prepare_one(replica_prepare_signed)
        .unwrap();
    let mut leader_prepare = util
        .recv_signed()
        .await
        .unwrap()
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;

    let correct_num = replica_prepare.high_vote.proposal.number.next();
    assert_eq!(correct_num, leader_prepare.proposal.number);

    let non_seq_num = correct_num.next();
    leader_prepare.proposal.number = non_seq_num;
    let leader_prepare_signed = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalNonSequentialNumber { correct_number, received_number, header }) => {
            assert_eq!(correct_number, correct_num);
            assert_eq!(received_number, non_seq_num);
            assert_eq!(header, leader_prepare.proposal);
        }
    );
}

#[tokio::test]
async fn leader_prepare_reproposal_without_quorum() {
    let mut util = UTHarness::new_many().await;

    util.set_view(util.owner_as_view_leader());

    let mut leader_prepare = util
        .new_procedural_leader_prepare_many()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;

    let rng = RefCell::new(util.new_rng());
    leader_prepare.justification = util.new_prepare_qc_many(&|msg: &mut ReplicaPrepare| {
        let mut rng = rng.borrow_mut();
        msg.high_vote = rng.gen();
    });
    leader_prepare.proposal_payload = None;

    let leader_prepare = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));
    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::ReproposalWithoutQuorum));
}

#[tokio::test]
async fn leader_prepare_reproposal_when_finalized() {
    let mut util = UTHarness::new_one().await;

    let mut leader_prepare = util
        .new_procedural_leader_prepare_one()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.proposal_payload = None;
    let leader_prepare_signed = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(res, Err(LeaderPrepareError::ReproposalWhenFinalized));
}

#[tokio::test]
async fn leader_prepare_reproposal_invalid_block() {
    let mut util = UTHarness::new_one().await;

    let mut leader_prepare: LeaderPrepare = util
        .new_procedural_leader_prepare_one()
        .await
        .cast()
        .unwrap()
        .msg;

    let high_vote = util.rng().gen();
    leader_prepare.justification = util.new_prepare_qc(|msg: &mut ReplicaPrepare| {
        msg.high_vote = high_vote;
    });
    leader_prepare.proposal_payload = None;

    let leader_prepare = util
        .owner_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));

    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::ReproposalInvalidBlock));
}

#[tokio::test]
async fn leader_commit_sanity() {
    let mut util = UTHarness::new_many().await;

    util.set_view(util.owner_as_view_leader());

    let leader_commit = util.new_procedural_leader_commit_many().await;
    util.dispatch_leader_commit(leader_commit).await.unwrap();
}

#[tokio::test]
async fn leader_commit_sanity_yield_replica_prepare() {
    let mut util = UTHarness::new_one().await;

    let leader_commit = util.new_procedural_leader_commit_one().await;
    util.dispatch_leader_commit(leader_commit.clone())
        .await
        .unwrap();
    let replica_prepare = util
        .recv_signed()
        .await
        .unwrap()
        .cast::<ReplicaPrepare>()
        .unwrap()
        .msg;

    let leader_commit = leader_commit.cast::<LeaderCommit>().unwrap().msg;
    assert_matches!(
        replica_prepare,
        ReplicaPrepare {
            protocol_version,
            view,
            high_vote,
            high_qc,
        } => {
            assert_eq!(protocol_version, leader_commit.protocol_version);
            assert_eq!(view, leader_commit.justification.message.view.next());
            assert_eq!(high_vote, leader_commit.justification.message);
            assert_eq!(high_qc, leader_commit.justification)
        }
    );
}

#[tokio::test]
async fn leader_commit_incompatible_protocol_version() {
    let mut util = UTHarness::new_one().await;

    let incompatible_protocol_version = util.incompatible_protocol_version();
    let leader_commit = util.new_rnd_leader_commit(|msg| {
        msg.protocol_version = incompatible_protocol_version;
    });
    let res = util.dispatch_leader_commit(leader_commit).await;
    assert_matches!(
        res,
        Err(LeaderCommitError::IncompatibleProtocolVersion { message_version, local_version }) => {
            assert_eq!(message_version, incompatible_protocol_version);
            assert_eq!(local_version, util.protocol_version());
        }
    )
}

#[tokio::test]
async fn leader_commit_invalid_leader() {
    let mut util = UTHarness::new_with(2).await;

    let current_view_leader = util.view_leader(util.current_replica_view());
    assert_ne!(current_view_leader, util.owner_key().public());

    let leader_commit = util.new_rnd_leader_commit(|_| {});
    let res = util.dispatch_leader_commit(leader_commit).await;
    assert_matches!(res, Err(LeaderCommitError::InvalidLeader { .. }));
}

#[tokio::test]
async fn leader_commit_invalid_sig() {
    let mut util = UTHarness::new_one().await;

    let mut leader_commit = util.new_rnd_leader_commit(|_| {});
    leader_commit.sig = util.rng().gen();
    let res = util.dispatch_leader_commit(leader_commit).await;
    assert_matches!(res, Err(LeaderCommitError::InvalidSignature { .. }));
}

#[tokio::test]
async fn leader_commit_invalid_commit_qc() {
    let mut util = UTHarness::new_one().await;

    let leader_commit = util.new_rnd_leader_commit(|_| {});
    let res = util.dispatch_leader_commit(leader_commit).await;
    assert_matches!(res, Err(LeaderCommitError::InvalidJustification { .. }));
}
