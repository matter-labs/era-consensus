use crate::{
    inner::ConsensusInner,
    leader::{
        ReplicaCommitError, ReplicaPrepareError,
        ReplicaPrepareError::{HighQCOfFutureView, InvalidHighQC},
    },
    replica::LeaderPrepareError,
    testonly::{make_genesis, ut_harness::UTHarness},
};
use assert_matches::assert_matches;
use rand::Rng;
use zksync_consensus_crypto::bn254::{Error::SignatureVerificationFailure, SecretKey};
use zksync_consensus_roles::{
    validator,
    validator::{
        BlockHeaderHash, CommitQC, ConsensusMsg, LeaderCommit, LeaderPrepare, Payload, Phase,
        PrepareQC, ProtocolVersion, ReplicaCommit, ReplicaPrepare, ViewNumber, CURRENT_VERSION,
    },
};

/// ## Tests coverage
///
/// - [x] replica_prepare_sanity
/// - [x] replica_prepare_sanity_yield_leader_prepare
/// - [x] replica_prepare_old_view
/// - [x] replica_prepare_during_commit
/// - [x] replica_prepare_not_leader_in_view
/// - [x] replica_prepare_already_exists
/// - [x] replica_prepare_num_received_below_threshold
/// - [x] replica_prepare_invalid_sig
/// - [x] replica_prepare_invalid_commit_qc
/// - [x] replica_prepare_high_qc_of_current_view
/// - [x] replica_prepare_high_qc_of_future_view
/// - [x] replica_prepare_non_validator_signer
/// -
/// - [x] leader_prepare_sanity
/// - [x] leader_prepare_sanity_yield_replica_commit
/// - [x] leader_prepare_invalid_leader
/// - [x] leader_prepare_old_view
/// - [x] leader_prepare_invalid_sig
/// - [x] leader_prepare_invalid_prepare_qc
/// - [x] leader_prepare_invalid_high_qc
/// - [x] leader_prepare_proposal_oversized_payload
/// - [x] leader_prepare_proposal_mismatched_payload
/// - [x] leader_prepare_proposal_when_previous_not_finalized
/// - [x] leader_prepare_proposal_invalid_parent_hash
/// - [x] leader_prepare_proposal_non_sequential_number
/// - [ ] leader_prepare_reproposal_without_quorum
/// - [x] leader_prepare_reproposal_when_finalized
/// - [ ] leader_prepare_reproposal_invalid_block
/// -
/// - [x] replica_commit_sanity
/// - [x] replica_commit_sanity_yield_leader_commit
/// - [x] replica_commit_old
/// - [x] replica_commit_not_leader_in_view
/// - [x] replica_commit_already_exists
/// - [x] replica_commit_num_received_below_threshold
/// - [x] replica_commit_invalid_sig
/// - [x] replica_commit_unexpected_proposal
/// - [x] replica_commit_protocol_version_mismatch
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
async fn replica_prepare_not_leader_in_view() {
    let mut util = UTHarness::new_with(2).await;

    let current_view_leader = util.view_leader(util.current_view());
    assert_ne!(current_view_leader, util.own_key().public());

    let replica_prepare = util.new_replica_prepare(|_| {});
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

    let replica_prepare = util.new_replica_prepare(|_| {});
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
async fn replica_prepare_invalid_sig() {
    let mut util = UTHarness::new().await;

    let mut replica_prepare = util.new_replica_prepare(|_| {});
    replica_prepare.sig = util.rng().gen();
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(res, Err(ReplicaPrepareError::InvalidSignature(_)));
}

#[tokio::test]
async fn replica_prepare_invalid_commit_qc() {
    let mut util = UTHarness::new().await;

    let junk = util.rng().gen::<CommitQC>();
    let replica_prepare = util.new_replica_prepare(|msg| msg.high_qc = junk);
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(res, Err(InvalidHighQC(..)));
}

#[tokio::test]
async fn replica_prepare_high_qc_of_current_view() {
    let mut util = UTHarness::new().await;

    let view = ViewNumber(1);
    let qc_view = ViewNumber(1);

    util.set_view(view);
    let qc = util.new_commit_qc(qc_view);
    let replica_prepare = util.new_replica_prepare(|msg| msg.high_qc = qc);
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(
        res,
        Err(HighQCOfFutureView{ high_qc_view, current_view }) => {
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
    let qc = util.new_commit_qc(qc_view);
    let replica_prepare = util.new_replica_prepare(|msg| msg.high_qc = qc);
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(
        res,
        Err(HighQCOfFutureView{ high_qc_view, current_view }) => {
            assert_eq!(high_qc_view, qc_view);
            assert_eq!(current_view, view);
        }
    );
}

#[tokio::test]
async fn replica_prepare_non_validator_signer() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_view(view);
    assert_eq!(util.view_leader(view), util.key_at(0).public());

    let replica_prepare = util.new_replica_prepare(|_| {});
    let _ = util.dispatch_replica_prepare(replica_prepare.clone());

    let non_validator: validator::SecretKey = util.rng().gen();
    let replica_prepare = non_validator.sign_msg(replica_prepare.msg);
    util.dispatch_replica_prepare(replica_prepare).unwrap();
    // PANICS:
    // "Couldn't create justification from valid replica messages!: Message signer isn't in the validator set"
}

#[tokio::test]
async fn leader_prepare_sanity() {
    let mut util = UTHarness::new().await;

    let leader_prepare = util.new_procedural_leader_prepare().await;
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();
}

#[tokio::test]
async fn leader_prepare_sanity_yield_replica_commit() {
    let mut util = UTHarness::new().await;

    let leader_prepare = util.new_procedural_leader_prepare().await;
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();
    util.recv_signed()
        .await
        .unwrap()
        .cast::<ReplicaCommit>()
        .unwrap();
}

#[tokio::test]
async fn leader_prepare_invalid_leader() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_view(view);
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
    let msg = util.recv_signed().await.unwrap();
    let mut leader_prepare = msg.cast::<LeaderPrepare>().unwrap().msg;

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
async fn leader_prepare_old_view() {
    let mut util = UTHarness::new().await;

    let mut leader_prepare = util
        .new_procedural_leader_prepare()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.view = util.current_view().prev();
    let leader_prepare = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));

    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::Old { current_view, current_phase }) => {
            assert_eq!(current_view, util.current_view());
            assert_eq!(current_phase, util.current_phase());
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
async fn leader_prepare_invalid_prepare_qc() {
    let mut util = UTHarness::new().await;

    let mut leader_prepare = util
        .new_procedural_leader_prepare()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.justification = util.rng().gen::<PrepareQC>();
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

#[tokio::test]
async fn leader_prepare_invalid_high_qc() {
    let mut util = UTHarness::new().await;

    let mut replica_prepare = util
        .new_replica_prepare(|_| {})
        .cast::<ReplicaPrepare>()
        .unwrap()
        .msg;
    replica_prepare.high_qc = util.rng().gen();

    let mut leader_prepare = util
        .new_procedural_leader_prepare()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.justification = util.new_prepare_qc(&replica_prepare);
    let leader_prepare = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));

    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::InvalidHighQC(_)))
}

#[tokio::test]
async fn leader_prepare_proposal_oversized_payload() {
    let mut util = UTHarness::new().await;
    let payload_oversize = ConsensusInner::PAYLOAD_MAX_SIZE + 1;
    let mut payload_vec = Vec::with_capacity(payload_oversize);
    payload_vec.resize(payload_oversize, 0);

    let mut leader_prepare = util
        .new_procedural_leader_prepare()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.proposal_payload = Some(Payload(payload_vec));
    let leader_prepare_signed = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalOversizedPayload{ payload_size, header }) => {
            assert_eq!(payload_size, payload_oversize);
            assert_eq!(header, leader_prepare.proposal);
        }
    )
}

#[tokio::test]
async fn leader_prepare_proposal_mismatched_payload() {
    let mut util = UTHarness::new().await;

    let mut leader_prepare = util
        .new_procedural_leader_prepare()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.proposal_payload = Some(util.rng().gen());
    let leader_prepare_signed = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(res, Err(LeaderPrepareError::ProposalMismatchedPayload))
}

#[tokio::test]
async fn leader_prepare_proposal_when_previous_not_finalized() {
    let mut util = UTHarness::new().await;

    let replica_prepare = util.new_replica_prepare(|_| {});
    util.dispatch_replica_prepare(replica_prepare.clone())
        .unwrap();
    let mut leader_prepare = util
        .recv_signed()
        .await
        .unwrap()
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;

    let mut replica_prepare = replica_prepare.cast::<ReplicaPrepare>().unwrap().msg;
    replica_prepare.high_vote = util.rng().gen();
    leader_prepare.justification = util.new_prepare_qc(&replica_prepare);

    let leader_prepare_signed = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalWhenPreviousNotFinalized)
    )
}

#[tokio::test]
async fn leader_prepare_proposal_invalid_parent_hash() {
    let mut util = UTHarness::new().await;

    let replica_prepare_signed = util.new_replica_prepare(|_| {});
    let replica_prepare = replica_prepare_signed
        .clone()
        .cast::<ReplicaPrepare>()
        .unwrap()
        .msg;
    util.dispatch_replica_prepare(replica_prepare_signed.clone())
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
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalInvalidParentHash { correct_parent_hash, received_parent_hash, header }) => {
            assert_eq!(correct_parent_hash, replica_prepare.high_vote.proposal.hash());
            assert_eq!(received_parent_hash, junk);
            assert_eq!(header, leader_prepare.proposal);
        }
    )
}

#[tokio::test]
async fn leader_prepare_proposal_non_sequential_number() {
    let mut util = UTHarness::new().await;

    let replica_prepare_signed = util.new_replica_prepare(|_| {});
    let replica_prepare = replica_prepare_signed
        .clone()
        .cast::<ReplicaPrepare>()
        .unwrap()
        .msg;
    util.dispatch_replica_prepare(replica_prepare_signed)
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
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalNonSequentialNumber { correct_number, received_number, header }) => {
            assert_eq!(correct_number, correct_num);
            assert_eq!(received_number, non_seq_num);
            assert_eq!(header, leader_prepare.proposal);
        }
    )
}

#[tokio::test]
async fn leader_prepare_reproposal_without_quorum() {
    todo!()
    // let mut util = UTHarness::new().await;
    //
    // let mut leader_prepare = util.new_procedural_leader_prepare().await
    //     .cast::<LeaderPrepare>().unwrap().msg;
    // leader_prepare.justification = util.new_empty_prepare_qc();
    // leader_prepare.proposal_payload = None;
    // let leader_prepare = util.own_key().sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));
    //
    // let res = util.dispatch_leader_prepare(leader_prepare).await;
    // assert_matches!(
    //     res,
    //     Err(LeaderPrepareError::ReproposalWithoutQuorum)
    // )
}

#[tokio::test]
async fn leader_prepare_reproposal_when_finalized() {
    let mut util = UTHarness::new().await;

    let mut leader_prepare = util
        .new_procedural_leader_prepare()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.proposal_payload = None;
    let leader_prepare_signed = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(res, Err(LeaderPrepareError::ReproposalWhenFinalized))
}

#[tokio::test]
async fn leader_prepare_reproposal_invalid_block() {
    let mut util = UTHarness::new().await;

    let mut leader_prepare = util
        .new_procedural_leader_prepare()
        .await
        .cast::<LeaderPrepare>()
        .unwrap()
        .msg;
    leader_prepare.proposal = util.rng().gen();
    let leader_prepare_signed = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare.clone()));

    let res = util.dispatch_leader_prepare(leader_prepare_signed).await;
    assert_matches!(res, Err(LeaderPrepareError::ReproposalInvalidBlock))
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
    replica_commit.view = util.current_view().prev();
    let replica_commit = util
        .own_key()
        .sign_msg(ConsensusMsg::ReplicaCommit(replica_commit));

    let res = util.dispatch_replica_commit(replica_commit);
    assert_matches!(
    res,
    Err(ReplicaCommitError::Old { current_view, current_phase }) => {
        assert_eq!(current_view, util.current_view());
        assert_eq!(current_phase, util.current_phase());
    });
}

#[tokio::test]
async fn replica_commit_not_leader_in_view() {
    let mut util = UTHarness::new_with(2).await;

    let current_view_leader = util.view_leader(util.current_view());
    assert_ne!(current_view_leader, util.own_key().public());

    let replica_commit = util.new_replica_commit();
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

    let replica_prepare_one = util.new_replica_prepare(|_| {});
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

    let replica_prepare_one = util.new_replica_prepare(|_| {});
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

    let mut replica_commit = util.new_replica_commit();
    replica_commit.sig = util.rng().gen();
    let res = util.dispatch_replica_commit(replica_commit);
    assert_matches!(res, Err(ReplicaCommitError::InvalidSignature(..)));
}

#[tokio::test]
async fn replica_commit_unexpected_proposal() {
    let mut util = UTHarness::new().await;

    let replica_commit = util.new_replica_commit();
    let res = util.dispatch_replica_commit(replica_commit);
    assert_matches!(res, Err(ReplicaCommitError::UnexpectedProposal));
}

#[tokio::test]
async fn replica_commit_protocol_version_mismatch() {
    let mut util = UTHarness::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.own_key().public());

    let replica_prepare_one = util.new_replica_prepare(|_| {});
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
    util.dispatch_replica_commit(replica_commit_two.into())
        .unwrap();
    // PANICS:
    // "Couldn't create justification from valid replica messages!: CommitQC can only be created from votes for the same message."
}