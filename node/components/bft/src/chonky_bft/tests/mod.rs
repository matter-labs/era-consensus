use crate::chonky_bft::testonly::UTHarness;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator;

mod commit;
mod new_view;
mod proposal;
//mod proposer;
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
async fn block_production_timeout_reproposal() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_replica_commit(ctx).await;
        let mut timeout = util.new_replica_timeout(ctx).await;

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

/// Testing liveness after the network becomes idle with replica in commit phase.
#[tokio::test]
async fn block_production_timeout_in_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.new_replica_commit(ctx).await;

        // Replica is in `Phase::Commit`, but should still accept messages from newer views.
        assert_eq!(util.replica.phase, validator::Phase::Commit);
        util.produce_block_after_timeout(ctx).await;

        Ok(())
    })
    .await
    .unwrap();
}

/// Testing liveness after the network becomes idle with replica having some cached commit messages for the current view.
#[tokio::test]
async fn block_production_timeout_some_commits() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_replica_commit(ctx).await;
        assert!(util
            .process_replica_commit(ctx, util.owner_key().sign_msg(replica_commit))
            .await
            .unwrap()
            .is_none());

        // Replica is in `Phase::Commit`, but should still accept prepares from newer views.
        assert_eq!(util.replica.phase, validator::Phase::Commit);
        util.produce_block_after_timeout(ctx).await;

        Ok(())
    })
    .await
    .unwrap();
}
