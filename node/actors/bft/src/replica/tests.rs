use super::{
    leader_commit::Error as LeaderCommitError, leader_prepare::Error as LeaderPrepareError,
};
use crate::{
    inner::ConsensusInner, leader::ReplicaPrepareError, testonly, testonly::ut_harness::UTHarness,
};
use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::{ctx, scope, testonly::abort_on_panic, time};
use zksync_consensus_network::io::{ConsensusInputMessage, Target};
use zksync_consensus_roles::validator::{
    self, BlockHeaderHash, ConsensusMsg, LeaderPrepare, Payload, PrepareQC, ReplicaCommit,
    ReplicaPrepare, ViewNumber,
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
/// - [x] replica_prepare_non_validator_signer // FAILS
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
/// - [x] replica_commit_protocol_version_mismatch // FAILS
/// -
/// - [x] leader_commit_sanity
/// - [x] leader_commit_sanity_yield_replica_prepare // FAILS
/// - [x] leader_commit_invalid_leader
/// - [x] leader_commit_old
/// - [x] leader_commit_invalid_sig
/// - [x] leader_commit_invalid_commit_qc
///

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

    let replica_prepare_one = util.new_current_replica_prepare(|_| {});
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
    leader_prepare.view = util.current_replica_view().prev();
    let leader_prepare = util
        .own_key()
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
    let mut util = UTHarness::new().await;

    let mut leader_prepare = util.new_rnd_leader_prepare(|_| {});
    leader_prepare.sig = util.rng().gen();

    let res = util.dispatch_leader_prepare(leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::InvalidSignature(..)));
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
        .new_current_replica_prepare(|_| {})
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

    let high_qc = util.rng().gen();
    leader_prepare.justification = util.new_prepare_qc(|msg| msg.high_qc = high_qc);
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
    let payload_vec = vec![0; payload_oversize];

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

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    util.dispatch_replica_prepare(replica_prepare.clone())
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

    let replica_prepare_signed = util.new_current_replica_prepare(|_| {});
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

    let replica_prepare_signed = util.new_current_replica_prepare(|_| {});
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

#[ignore = "unimplemented"]
#[tokio::test]
async fn leader_prepare_reproposal_without_quorum() {
    // let mut util = UTHarness::new().await;
    //
    // let mut leader_prepare = util
    //     .new_procedural_leader_prepare()
    //     .await
    //     .cast::<LeaderPrepare>()
    //     .unwrap()
    //     .msg;
    // leader_prepare.justification = util.new_empty_prepare_qc();
    // leader_prepare.proposal_payload = None;
    // let leader_prepare = util
    //     .own_key()
    //     .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));
    //
    // let res = util.dispatch_leader_prepare(leader_prepare).await;
    // assert_matches!(res, Err(LeaderPrepareError::ReproposalWithoutQuorum))
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

#[ignore = "unimplemented"]
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
async fn leader_commit_sanity() {
    let mut util = UTHarness::new().await;

    let leader_commit = util.new_procedural_leader_commit().await;
    util.dispatch_leader_commit(leader_commit).await.unwrap();
}

#[tokio::test]
async fn leader_commit_sanity_yield_replica_prepare() {
    let mut util = UTHarness::new().await;

    let leader_commit = util.new_procedural_leader_commit().await;
    util.dispatch_leader_commit(leader_commit).await.unwrap();
    util.recv_signed() // FAILS
        .await
        .unwrap()
        .cast::<ReplicaPrepare>()
        .unwrap();
}

#[tokio::test]
async fn leader_commit_invalid_leader() {
    let mut util = UTHarness::new_with(2).await;

    let current_view_leader = util.view_leader(util.current_replica_view());
    assert_ne!(current_view_leader, util.own_key().public());

    let leader_commit = util.new_rnd_leader_commit(|_| {});
    let res = util.dispatch_leader_commit(leader_commit).await;
    assert_matches!(res, Err(LeaderCommitError::InvalidLeader { .. }));
}

#[tokio::test]
async fn leader_commit_invalid_sig() {
    let mut util = UTHarness::new().await;

    let mut leader_commit = util.new_rnd_leader_commit(|_| {});
    leader_commit.sig = util.rng().gen();
    let res = util.dispatch_leader_commit(leader_commit).await;
    assert_matches!(res, Err(LeaderCommitError::InvalidSignature { .. }));
}

#[tokio::test]
async fn leader_commit_invalid_commit_qc() {
    let mut util = UTHarness::new().await;

    let leader_commit = util.new_rnd_leader_commit(|_| {});
    let res = util.dispatch_leader_commit(leader_commit).await;
    assert_matches!(res, Err(LeaderCommitError::InvalidJustification { .. }));
}

#[tokio::test]
async fn start_new_view_not_leader() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::ManualClock::new());
    let rng = &mut ctx.rng();

    let keys: Vec<_> = (0..4).map(|_| rng.gen()).collect();
    let (genesis, val_set) = testonly::make_genesis(&keys, Payload(vec![]));
    let (mut consensus, mut pipe) =
        testonly::make_consensus(ctx, &keys[0], &val_set, &genesis).await;
    // TODO: this test assumes a specific implementation of the leader schedule.
    // Make it leader-schedule agnostic (use epoch to select a specific view).
    consensus.replica.view = ViewNumber(1);
    consensus.replica.high_qc = rng.gen();
    consensus.replica.high_qc.message.view = ViewNumber(0);

    scope::run!(ctx, |ctx, s| {
        s.spawn(async {
            consensus
                .replica
                .start_new_view(ctx, &consensus.inner)
                .await
                .unwrap();
            Ok(())
        })
        .join(ctx)
    })
    .await
    .unwrap();

    let test_new_view_msg = ConsensusInputMessage {
        message: consensus
            .inner
            .secret_key
            .sign_msg(ConsensusMsg::ReplicaPrepare(ReplicaPrepare {
                protocol_version: validator::CURRENT_VERSION,
                view: consensus.replica.view,
                high_vote: consensus.replica.high_vote,
                high_qc: consensus.replica.high_qc.clone(),
            })),
        recipient: Target::Validator(consensus.inner.view_leader(consensus.replica.view)),
    };

    assert_eq!(pipe.recv(ctx).await.unwrap(), test_new_view_msg.into());
    assert!(consensus.replica.timeout_deadline < time::Deadline::Infinite);
}
