use super::{leader_commit, leader_prepare};
use crate::{inner::ConsensusInner, testonly::ut_harness::UTHarness};
use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator::{
    self, CommitQC, Payload, PrepareQC, ReplicaCommit, ReplicaPrepare, ViewNumber,
};

#[tokio::test]
async fn leader_prepare_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;
    let leader_prepare = util.new_leader_prepare(ctx).await;
    util.process_leader_prepare(ctx, leader_prepare)
        .await
        .unwrap();
}

#[tokio::test]
async fn leader_prepare_reproposal_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;
    util.new_replica_commit(ctx).await;
    util.process_replica_timeout(ctx).await;
    let leader_prepare = util.new_leader_prepare(ctx).await;
    assert!(leader_prepare.msg.proposal_payload.is_none());
    util.process_leader_prepare(ctx, leader_prepare)
        .await
        .unwrap();
}

#[tokio::test]
async fn leader_prepare_incompatible_protocol_version() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;

    let incompatible_protocol_version = util.incompatible_protocol_version();
    let mut leader_prepare = util.new_leader_prepare(ctx).await.msg;
    leader_prepare.protocol_version = incompatible_protocol_version;
    let res = util
        .process_leader_prepare(ctx, util.owner_key().sign_msg(leader_prepare))
        .await;
    assert_matches!(
        res,
        Err(leader_prepare::Error::IncompatibleProtocolVersion { message_version, local_version }) => {
            assert_eq!(message_version, incompatible_protocol_version);
            assert_eq!(local_version, util.protocol_version());
        }
    )
}

#[tokio::test]
async fn leader_prepare_sanity_yield_replica_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;

    let leader_prepare = util.new_leader_prepare(ctx).await;
    let replica_commit = util
        .process_leader_prepare(ctx, leader_prepare.clone())
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
    let mut util = UTHarness::new(ctx, 2).await;

    let view = ViewNumber(2);
    util.set_view(view);
    assert_eq!(util.view_leader(view), util.keys[0].public());

    let replica_prepare = util.new_replica_prepare(|_| {});
    assert!(util
        .process_replica_prepare(ctx, replica_prepare.clone())
        .await
        .unwrap()
        .is_none());

    let replica_prepare = util.keys[1].sign_msg(replica_prepare.msg);
    let mut leader_prepare = util
        .process_replica_prepare(ctx, replica_prepare)
        .await
        .unwrap()
        .unwrap()
        .msg;
    leader_prepare.view = leader_prepare.view.next();
    assert_ne!(util.view_leader(leader_prepare.view), util.keys[0].public());

    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(
        res,
        Err(leader_prepare::Error::InvalidLeader { correct_leader, received_leader }) => {
            assert_eq!(correct_leader, util.keys[1].public());
            assert_eq!(received_leader, util.keys[0].public());
        }
    );
}

#[tokio::test]
async fn leader_prepare_old_view() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let mut leader_prepare = util.new_leader_prepare(ctx).await.msg;
    leader_prepare.view = util.replica.view.prev();
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(
        res,
        Err(leader_prepare::Error::Old { current_view, current_phase }) => {
            assert_eq!(current_view, util.replica.view);
            assert_eq!(current_phase, util.replica.phase);
        }
    );
}

/// Tests that `WriteBlockStore::verify_payload` is applied before signing a vote.
#[tokio::test]
async fn leader_prepare_invalid_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let leader_prepare = util.new_leader_prepare(ctx).await;

    // Insert a finalized block to the storage.
    // Default implementation of verify_payload() fails if
    // head block number >= proposal block number.
    let block = validator::FinalBlock {
        header: leader_prepare.msg.proposal,
        payload: leader_prepare.msg.proposal_payload.clone().unwrap(),
        justification: CommitQC::from(
            &[util.keys[0].sign_msg(ReplicaCommit {
                protocol_version: util.protocol_version(),
                view: util.replica.view,
                proposal: leader_prepare.msg.proposal,
            })],
            &util.validator_set(),
        )
        .unwrap(),
    };
    util.replica.storage.put_block(ctx, &block).await.unwrap();

    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(res, Err(leader_prepare::Error::ProposalInvalidPayload(..)));
}

#[tokio::test]
async fn leader_prepare_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let mut leader_prepare = util.new_leader_prepare(ctx).await;
    leader_prepare.sig = ctx.rng().gen();
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(res, Err(leader_prepare::Error::InvalidSignature(..)));
}

#[tokio::test]
async fn leader_prepare_invalid_prepare_qc() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let mut leader_prepare = util.new_leader_prepare(ctx).await.msg;
    leader_prepare.justification = ctx.rng().gen();
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(res, Err(leader_prepare::Error::InvalidPrepareQC(_)));
}

#[tokio::test]
async fn leader_prepare_invalid_high_qc() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let mut leader_prepare = util.new_leader_prepare(ctx).await.msg;
    leader_prepare.justification = util.new_prepare_qc(|msg| msg.high_qc = ctx.rng().gen());
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(res, Err(leader_prepare::Error::InvalidHighQC(_)));
}

#[tokio::test]
async fn leader_prepare_proposal_oversized_payload() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let payload_oversize = ConsensusInner::PAYLOAD_MAX_SIZE + 1;
    let payload_vec = vec![0; payload_oversize];
    let mut leader_prepare = util.new_leader_prepare(ctx).await.msg;
    leader_prepare.proposal_payload = Some(Payload(payload_vec));
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util
        .process_leader_prepare(ctx, leader_prepare.clone())
        .await;
    assert_matches!(
        res,
        Err(leader_prepare::Error::ProposalOversizedPayload{ payload_size, header }) => {
            assert_eq!(payload_size, payload_oversize);
            assert_eq!(header, leader_prepare.msg.proposal);
        }
    );
}

#[tokio::test]
async fn leader_prepare_proposal_mismatched_payload() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let mut leader_prepare = util.new_leader_prepare(ctx).await.msg;
    leader_prepare.proposal_payload = Some(ctx.rng().gen());
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(res, Err(leader_prepare::Error::ProposalMismatchedPayload));
}

#[tokio::test]
async fn leader_prepare_proposal_when_previous_not_finalized() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let replica_prepare = util.new_replica_prepare(|_| {});
    let mut leader_prepare = util
        .process_replica_prepare(ctx, replica_prepare)
        .await
        .unwrap()
        .unwrap()
        .msg;
    leader_prepare.justification = util.new_prepare_qc(|msg| msg.high_vote = ctx.rng().gen());
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(
        res,
        Err(leader_prepare::Error::ProposalWhenPreviousNotFinalized)
    );
}

#[tokio::test]
async fn leader_prepare_proposal_invalid_parent_hash() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let replica_prepare = util.new_replica_prepare(|_| {});
    let mut leader_prepare = util
        .process_replica_prepare(ctx, replica_prepare.clone())
        .await
        .unwrap()
        .unwrap()
        .msg;
    leader_prepare.proposal.parent = ctx.rng().gen();
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util
        .process_leader_prepare(ctx, leader_prepare.clone())
        .await;
    assert_matches!(
        res,
        Err(leader_prepare::Error::ProposalInvalidParentHash {
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
    let mut util = UTHarness::new(ctx, 1).await;
    let replica_prepare = util.new_replica_prepare(|_| {});
    let mut leader_prepare = util
        .process_replica_prepare(ctx, replica_prepare.clone())
        .await
        .unwrap()
        .unwrap()
        .msg;
    let correct_num = replica_prepare.msg.high_vote.proposal.number.next();
    assert_eq!(correct_num, leader_prepare.proposal.number);

    let non_seq_num = correct_num.next();
    leader_prepare.proposal.number = non_seq_num;
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util
        .process_leader_prepare(ctx, leader_prepare.clone())
        .await;
    assert_matches!(
        res,
        Err(leader_prepare::Error::ProposalNonSequentialNumber { correct_number, received_number, header }) => {
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
    let replica_prepare = util.new_replica_prepare(|_| {}).msg;
    let mut leader_prepare = util
        .process_replica_prepare_all(ctx, replica_prepare.clone())
        .await
        .msg;

    // Turn leader_prepare into an unjustified reproposal.
    let replica_prepares: Vec<_> = util
        .keys
        .iter()
        .map(|k| {
            let mut msg = replica_prepare.clone();
            msg.high_vote = rng.gen();
            k.sign_msg(msg)
        })
        .collect();
    leader_prepare.justification =
        PrepareQC::from(&replica_prepares, &util.validator_set()).unwrap();
    leader_prepare.proposal_payload = None;

    let leader_prepare = util.keys[0].sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(res, Err(leader_prepare::Error::ReproposalWithoutQuorum));
}

#[tokio::test]
async fn leader_prepare_reproposal_when_finalized() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let mut leader_prepare = util.new_leader_prepare(ctx).await.msg;
    leader_prepare.proposal_payload = None;
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(res, Err(leader_prepare::Error::ReproposalWhenFinalized));
}

#[tokio::test]
async fn leader_prepare_reproposal_invalid_block() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let mut leader_prepare = util.new_leader_prepare(ctx).await.msg;
    leader_prepare.justification = util.new_prepare_qc(|msg| msg.high_vote = ctx.rng().gen());
    leader_prepare.proposal_payload = None;
    let leader_prepare = util.owner_key().sign_msg(leader_prepare);
    let res = util.process_leader_prepare(ctx, leader_prepare).await;
    assert_matches!(res, Err(leader_prepare::Error::ReproposalInvalidBlock));
}

#[tokio::test]
async fn leader_commit_sanity() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;
    let leader_commit = util.new_leader_commit(ctx).await;
    util.process_leader_commit(ctx, leader_commit)
        .await
        .unwrap();
}

#[tokio::test]
async fn leader_commit_sanity_yield_replica_prepare() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;
    let leader_commit = util.new_leader_commit(ctx).await;
    let replica_prepare = util
        .process_leader_commit(ctx, leader_commit.clone())
        .await
        .unwrap();
    assert_eq!(
        replica_prepare.msg,
        ReplicaPrepare {
            protocol_version: leader_commit.msg.protocol_version,
            view: leader_commit.msg.justification.message.view.next(),
            high_vote: leader_commit.msg.justification.message,
            high_qc: leader_commit.msg.justification,
        }
    );
}

#[tokio::test]
async fn leader_commit_incompatible_protocol_version() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 1).await;

    let incompatible_protocol_version = util.incompatible_protocol_version();
    let mut leader_commit = util.new_leader_commit(ctx).await.msg;
    leader_commit.protocol_version = incompatible_protocol_version;
    let res = util
        .process_leader_commit(ctx, util.owner_key().sign_msg(leader_commit))
        .await;
    assert_matches!(
        res,
        Err(leader_commit::Error::IncompatibleProtocolVersion { message_version, local_version }) => {
            assert_eq!(message_version, incompatible_protocol_version);
            assert_eq!(local_version, util.protocol_version());
        }
    )
}

#[tokio::test]
async fn leader_commit_invalid_leader() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new(ctx, 2).await;
    let current_view_leader = util.view_leader(util.replica.view);
    assert_ne!(current_view_leader, util.owner_key().public());

    let leader_commit = util.new_leader_commit(ctx).await.msg;
    let res = util
        .process_leader_commit(ctx, util.keys[1].sign_msg(leader_commit))
        .await;
    assert_matches!(res, Err(leader_commit::Error::InvalidLeader { .. }));
}

#[tokio::test]
async fn leader_commit_invalid_sig() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut util = UTHarness::new(ctx, 1).await;
    let mut leader_commit = util.new_leader_commit(ctx).await;
    leader_commit.sig = rng.gen();
    let res = util.process_leader_commit(ctx, leader_commit).await;
    assert_matches!(res, Err(leader_commit::Error::InvalidSignature { .. }));
}

#[tokio::test]
async fn leader_commit_invalid_commit_qc() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut util = UTHarness::new(ctx, 1).await;
    let mut leader_commit = util.new_leader_commit(ctx).await.msg;
    leader_commit.justification = rng.gen();
    let res = util
        .process_leader_commit(ctx, util.owner_key().sign_msg(leader_commit))
        .await;
    assert_matches!(res, Err(leader_commit::Error::InvalidJustification { .. }));
}
