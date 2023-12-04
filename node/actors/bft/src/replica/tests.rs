use super::{
    leader_commit::Error as LeaderCommitError, leader_prepare::Error as LeaderPrepareError,
};
use zksync_concurrency::ctx;
use crate::{inner::ConsensusInner, leader::ReplicaPrepareError, testonly::ut_harness::UTHarness};
use assert_matches::assert_matches;
use rand::Rng;
use zksync_consensus_roles::validator::{
    self,
    LeaderPrepare, Payload, ReplicaCommit,
    ReplicaPrepare, ViewNumber, CommitQC,
};

#[tokio::test]
async fn leader_prepare_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;
    util.set_view(util.owner_as_view_leader());
    let leader_prepare = util.new_procedural_leader_prepare(ctx).await;
    util.process_leader_prepare(ctx,leader_prepare).await.unwrap();
}

#[tokio::test]
async fn leader_prepare_reproposal_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;
    util.set_view(util.owner_as_view_leader());
    let replica_prepare = util.new_unfinalized_replica_prepare().msg;
    let leader_prepare = util.process_replica_prepare_all(ctx,replica_prepare.clone()).await;

    assert_matches!(
        &leader_prepare.msg,
        LeaderPrepare {proposal_payload, .. } => {
            assert!(proposal_payload.is_none());
        }
    );

    util.process_leader_prepare(ctx,leader_prepare)
        .await
        .unwrap();
}

#[tokio::test]
async fn leader_prepare_sanity_yield_replica_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;

    let leader_prepare = util.new_procedural_leader_prepare(ctx).await;
    let replica_commit = util.process_leader_prepare(ctx,leader_prepare.clone())
        .await
        .unwrap();
    assert_eq!(
        replica_commit.msg,
        ReplicaCommit {
            protocol_version: leader_prepare.msg.protocol_version,
            view: leader_prepare.msg.view,
            proposal: leader_prepare.msg.proposal,
        }
    );
}

#[tokio::test]
async fn leader_prepare_invalid_leader() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,2).await;

    let view = ViewNumber(2);
    util.set_view(view);
    assert_eq!(util.view_leader(view), util.keys[0].public());

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let res = util.process_replica_prepare(ctx,replica_prepare.clone()).await;
    assert_matches!(
        res,
        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
            num_messages: 1,
            threshold: 2,
        })
    );

    let replica_prepare = util.keys[1].sign_msg(replica_prepare.msg);
    let mut leader_prepare = util.process_replica_prepare(ctx,replica_prepare).await.unwrap().unwrap().msg;
    leader_prepare.view = leader_prepare.view.next();
    assert_ne!(
        util.view_leader(leader_prepare.view),
        util.keys[0].public()
    );

    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::InvalidLeader { correct_leader, received_leader }) => {
            assert_eq!(correct_leader, util.keys[1].public());
            assert_eq!(received_leader, util.keys[0].public());
        }
    );
}

#[tokio::test]
async fn leader_prepare_old_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let mut leader_prepare = util.new_procedural_leader_prepare(ctx).await.msg;
    leader_prepare.view = util.consensus.replica.view.prev();
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::Old { current_view, current_phase }) => {
            assert_eq!(current_view, util.consensus.replica.view);
            assert_eq!(current_phase, util.consensus.replica.phase);
        }
    );
}

/// Tests that `WriteBlockStore::verify_payload` is applied before signing a vote.
#[tokio::test]
async fn leader_prepare_invalid_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let leader_prepare = util.new_procedural_leader_prepare(ctx).await;

    // Insert a finalized block to the storage.
    // Default implementation of verify_payload() fails if
    // head block number >= proposal block number.
    let block = validator::FinalBlock {
        header: leader_prepare.msg.proposal.clone(),
        payload: leader_prepare.msg.proposal_payload.clone().unwrap(),
        justification: CommitQC::from(
            &[util.keys[0].sign_msg(ReplicaCommit {
                protocol_version: validator::ProtocolVersion::EARLIEST,
                view: util.consensus.replica.view,
                proposal: leader_prepare.msg.proposal.clone(),
            })],
            &util.validator_set(),
        ).unwrap(),
    };
    util.consensus.replica.storage.put_block(ctx,&block).await.unwrap();

    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(res,Err(LeaderPrepareError::ProposalInvalidPayload(..)));
}

#[tokio::test]
async fn leader_prepare_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let mut leader_prepare = util.new_rnd_leader_prepare(&mut ctx.rng(),|_| {});
    leader_prepare.sig = ctx.rng().gen();
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::InvalidSignature(..)));
}

#[tokio::test]
async fn leader_prepare_invalid_prepare_qc() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let mut leader_prepare = util.new_procedural_leader_prepare(ctx).await.msg;
    leader_prepare.justification = ctx.rng().gen();
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::InvalidPrepareQC(_))
    );
}

#[tokio::test]
async fn leader_prepare_invalid_high_qc() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await; 
    let mut leader_prepare = util.new_procedural_leader_prepare(ctx).await.msg;
    leader_prepare.justification = util.new_prepare_qc(|msg| msg.high_qc = ctx.rng().gen());
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::InvalidHighQC(_)));
}

#[tokio::test]
async fn leader_prepare_proposal_oversized_payload() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let payload_oversize = ConsensusInner::PAYLOAD_MAX_SIZE + 1;
    let payload_vec = vec![0; payload_oversize];
    let mut leader_prepare = util.new_procedural_leader_prepare(ctx).await.msg;
    leader_prepare.proposal_payload = Some(Payload(payload_vec));
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare.clone()).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalOversizedPayload{ payload_size, header }) => {
            assert_eq!(payload_size, payload_oversize);
            assert_eq!(header, leader_prepare.msg.proposal);
        }
    );
}

#[tokio::test]
async fn leader_prepare_proposal_mismatched_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let mut leader_prepare = util.new_procedural_leader_prepare(ctx).await.msg;
    leader_prepare.proposal_payload = Some(ctx.rng().gen());
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::ProposalMismatchedPayload));
}

#[tokio::test]
async fn leader_prepare_proposal_when_previous_not_finalized() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let mut leader_prepare = util.process_replica_prepare(ctx,replica_prepare).await.unwrap().unwrap().msg;
    leader_prepare.justification = util.new_prepare_qc(|msg| msg.high_vote = ctx.rng().gen());
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalWhenPreviousNotFinalized)
    );
}

#[tokio::test]
async fn leader_prepare_proposal_invalid_parent_hash() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let replica_prepare = util.new_current_replica_prepare(|_| {}); 
    let mut leader_prepare = util.process_replica_prepare(ctx,replica_prepare.clone()).await.unwrap().unwrap().msg;
    leader_prepare.proposal.parent = ctx.rng().gen();
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare.clone()).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalInvalidParentHash {
            correct_parent_hash,
            received_parent_hash,
            header
        }) => {
            assert_eq!(correct_parent_hash, replica_prepare.msg.high_vote.proposal.hash());
            assert_eq!(received_parent_hash, leader_prepare.msg.proposal.parent);
            assert_eq!(header, leader_prepare.msg.proposal);
        }
    );
}

#[tokio::test]
async fn leader_prepare_proposal_non_sequential_number() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let mut leader_prepare = util.process_replica_prepare(ctx,replica_prepare.clone()).await.unwrap().unwrap().msg;
    let correct_num = replica_prepare.msg.high_vote.proposal.number.next();
    assert_eq!(correct_num, leader_prepare.proposal.number);

    let non_seq_num = correct_num.next();
    leader_prepare.proposal.number = non_seq_num;
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare.clone()).await;
    assert_matches!(
        res,
        Err(LeaderPrepareError::ProposalNonSequentialNumber { correct_number, received_number, header }) => {
            assert_eq!(correct_number, correct_num);
            assert_eq!(received_number, non_seq_num);
            assert_eq!(header, leader_prepare.msg.proposal);
        }
    );
}

#[tokio::test]
async fn leader_prepare_reproposal_without_quorum() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut util = UTHarness::new_many(ctx).await;
    util.set_view(util.owner_as_view_leader());
    let mut leader_prepare = util.new_procedural_leader_prepare(ctx).await.msg;
    leader_prepare.justification = util.new_prepare_qc_many(|msg| msg.high_vote = rng.gen());
    leader_prepare.proposal_payload = None;
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::ReproposalWithoutQuorum));
}

#[tokio::test]
async fn leader_prepare_reproposal_when_finalized() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let mut leader_prepare = util.new_procedural_leader_prepare(ctx).await.msg;
    leader_prepare.proposal_payload = None;
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::ReproposalWhenFinalized));
}

#[tokio::test]
async fn leader_prepare_reproposal_invalid_block() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let mut leader_prepare = util.new_procedural_leader_prepare(ctx).await.msg;
    leader_prepare.justification = util.new_prepare_qc(|msg| msg.high_vote = ctx.rng().gen());
    leader_prepare.proposal_payload = None;
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx,leader_prepare).await;
    assert_matches!(res, Err(LeaderPrepareError::ReproposalInvalidBlock));
}

#[tokio::test]
async fn leader_commit_sanity() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;
    util.set_view(util.owner_as_view_leader());
    let leader_commit = util.new_procedural_leader_commit(ctx).await;
    util.process_leader_commit(ctx,leader_commit).await.unwrap();
}

#[tokio::test]
async fn leader_commit_sanity_yield_replica_prepare() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,1).await;
    let leader_commit = util.new_procedural_leader_commit(ctx).await;
    let replica_prepare = util.process_leader_commit(ctx,leader_commit.clone()).await.unwrap();
    assert_eq!(
        replica_prepare.msg,
        ReplicaPrepare {
            protocol_version: leader_commit.msg.protocol_version,
            view: leader_commit.msg.justification.message.view.next(),
            high_vote: leader_commit.msg.justification.message.clone(),
            high_qc: leader_commit.msg.justification,
        }
    );
}

#[tokio::test]
async fn leader_commit_invalid_leader() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx,2).await;
    let current_view_leader = util.view_leader(util.consensus.replica.view);
    assert_ne!(current_view_leader, util.owner_key().public());

    let leader_commit = util.new_rnd_leader_commit(&mut ctx.rng(),|_| {});
    let res = util.process_leader_commit(ctx,leader_commit).await;
    assert_matches!(res, Err(LeaderCommitError::InvalidLeader { .. }));
}

#[tokio::test]
async fn leader_commit_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut util = UTHarness::new(ctx,1).await;
    let mut leader_commit = util.new_rnd_leader_commit(rng,|_| {});
    leader_commit.sig = rng.gen();
    let res = util.process_leader_commit(ctx,leader_commit).await;
    assert_matches!(res, Err(LeaderCommitError::InvalidSignature { .. }));
}

#[tokio::test]
async fn leader_commit_invalid_commit_qc() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut util = UTHarness::new(ctx,1).await;
    let leader_commit = util.new_rnd_leader_commit(rng,|_| {});
    let res = util.process_leader_commit(ctx,leader_commit).await;
    assert_matches!(res, Err(LeaderCommitError::InvalidJustification { .. }));
}
