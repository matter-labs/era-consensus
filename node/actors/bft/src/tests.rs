use crate::{
    misc::consensus_threshold,
    testonly::{ut_harness::UTHarness, Behavior, Network, Test},
};
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator::Phase;

async fn run_test(behavior: Behavior, network: Network) {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(1.));

    const NODES: usize = 11;
    let mut nodes = vec![behavior; NODES];
    for n in &mut nodes[0..consensus_threshold(NODES)] {
        *n = Behavior::Honest;
    }
    Test {
        network,
        nodes,
        blocks_to_finalize: 15,
    }
    .run(ctx)
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn honest_mock_network() {
    run_test(Behavior::Honest, Network::Mock).await
}

#[tokio::test(flavor = "multi_thread")]
async fn honest_real_network() {
    run_test(Behavior::Honest, Network::Real).await
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_mock_network() {
    run_test(Behavior::Offline, Network::Mock).await
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_real_network() {
    run_test(Behavior::Offline, Network::Real).await
}

#[tokio::test(flavor = "multi_thread")]
async fn random_mock_network() {
    run_test(Behavior::Random, Network::Mock).await
}

#[tokio::test(flavor = "multi_thread")]
async fn random_real_network() {
    run_test(Behavior::Random, Network::Real).await
}

#[tokio::test(flavor = "multi_thread")]
async fn byzantine_mock_network() {
    run_test(Behavior::Byzantine, Network::Mock).await
}

#[tokio::test(flavor = "multi_thread")]
async fn byzantine_real_network() {
    run_test(Behavior::Byzantine, Network::Real).await
}

// Testing liveness after the network becomes idle with leader having no cached prepare messages for the current view.
#[tokio::test]
async fn timeout_leader_no_prepares() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;

    util.new_replica_prepare(|_| {});
    util.produce_block_after_timeout(ctx).await;
}

/// Testing liveness after the network becomes idle with leader having some cached prepare messages for the current view.
#[tokio::test]
async fn timeout_leader_some_prepares() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;

    let replica_prepare = util.new_replica_prepare(|_| {});
    assert!(util
        .process_replica_prepare(ctx, replica_prepare)
        .await
        .is_err());
    util.produce_block_after_timeout(ctx).await;
}

/// Testing liveness after the network becomes idle with leader in commit phase.
#[tokio::test]
async fn timeout_leader_in_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;

    util.new_leader_prepare(ctx).await;
    // Leader is in `Phase::Commit`, but should still accept prepares from newer views.
    assert_eq!(util.consensus.leader.phase, Phase::Commit);
    util.produce_block_after_timeout(ctx).await;
}

/// Testing liveness after the network becomes idle with replica in commit phase.
#[tokio::test]
async fn timeout_replica_in_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;

    util.new_replica_commit(ctx).await;
    // Leader is in `Phase::Commit`, but should still accept prepares from newer views.
    assert_eq!(util.consensus.leader.phase, Phase::Commit);
    util.produce_block_after_timeout(ctx).await;
}

/// Testing liveness after the network becomes idle with leader having some cached commit messages for the current view.
#[tokio::test]
async fn timeout_leader_some_commits() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;

    let replica_commit = util.new_replica_commit(ctx).await;
    assert!(util
        .process_replica_commit(ctx, replica_commit)
        .await
        .is_err());
    // Leader is in `Phase::Commit`, but should still accept prepares from newer views.
    assert_eq!(util.leader_phase(), Phase::Commit);
    util.produce_block_after_timeout(ctx).await;
}

/// Testing liveness after the network becomes idle with leader in a consecutive prepare phase.
#[tokio::test]
async fn timeout_leader_in_consecutive_prepare() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let mut util = UTHarness::new_many(ctx).await;

    util.new_leader_commit(ctx).await;
    util.produce_block_after_timeout(ctx).await;
}
