use crate::chonky_bft::testonly::UTHarness;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator;

mod commit;
mod proposal;
mod timeout;

/// Sanity check of the happy path.
#[tokio::test]
async fn block_production() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.produce_block(ctx).await;

        Ok(())
    })
    .await
    .unwrap();
}

/// Sanity check of block production after timeout
#[tokio::test]
async fn block_production_timeout() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.produce_block_after_timeout(ctx).await;

        Ok(())
    })
    .await
    .unwrap();
}

/// Sanity check of block production with reproposal.
#[tokio::test]
async fn reproposal_block_production() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let proposal = util.new_leader_proposal(ctx).await;
        let replica_commit = util
            .process_leader_proposal(ctx, util.leader_key().sign_msg(proposal.clone()))
            .await
            .unwrap()
            .msg;

        let mut timeout = validator::ReplicaTimeout {
            view: replica_commit.view.clone(),
            high_vote: Some(replica_commit.clone()),
            high_qc: util.replica.high_commit_qc.clone(),
        };
        for i in 0..util.genesis().validators.subquorum_threshold() as usize {
            util.process_replica_timeout(ctx, util.keys[i].sign_msg(timeout.clone()))
                .await
                .unwrap();
        }
        timeout.high_vote = None;
        for i in util.genesis().validators.subquorum_threshold() as usize..util.keys.len() {
            let _ = util
                .process_replica_timeout(ctx, util.keys[i].sign_msg(timeout.clone()))
                .await;
        }

        assert!(util.replica.high_commit_qc.is_none());
        util.produce_block(ctx).await;
        assert_eq!(
            util.replica.high_commit_qc.unwrap().message.proposal,
            replica_commit.proposal
        );

        Ok(())
    })
    .await
    .unwrap();
}
