use crate::{
    leader::{ReplicaCommitError, ReplicaPrepareError},
    misc::consensus_threshold,
    testonly::{ut_harness::UTHarness, Behavior, Network, Test},
};
use assert_matches::assert_matches;
use zksync_concurrency::{ctx, testonly::abort_on_panic};
use zksync_consensus_roles::validator::Phase;

async fn run_test(behavior: Behavior, network: Network) {
    abort_on_panic();
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

/// Testing liveness after the network becomes idle with leader having no cached prepare messages for the current view.
#[tokio::test]
async fn timeout_leader_no_prepares() {
    let mut util = UTHarness::new_many().await;

    let base_replica_view = util.replica_view();
    let base_leader_view = util.leader_view();

    util.sim_timeout().await;

    assert_eq!(util.replica_view(), base_replica_view.next());
    assert_eq!(util.replica_phase(), Phase::Prepare);
    assert_eq!(util.leader_view(), base_leader_view);
    assert_eq!(util.leader_phase(), Phase::Prepare);

    util.check_recovery_after_timeout();
}

/// Testing liveness after the network becomes idle with leader having some cached prepare messages for the current view.
#[tokio::test]
async fn timeout_leader_some_prepares() {
    let mut util = UTHarness::new_many().await;

    let replica_prepare = util.new_current_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare_one(replica_prepare);
    assert_matches!(
        res,
        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
            num_messages,
            threshold,
        }) => {
            assert_eq!(num_messages, 1);
            assert_eq!(threshold, util.consensus_threshold())
        }
    );

    let base_replica_view = util.replica_view();
    let base_leader_view = util.leader_view();

    util.sim_timeout().await;

    assert_eq!(util.replica_view(), base_replica_view.next());
    assert_eq!(util.replica_phase(), Phase::Prepare);
    assert_eq!(util.leader_view(), base_leader_view);
    assert_eq!(util.leader_phase(), Phase::Prepare);

    util.check_recovery_after_timeout();
}

/// Testing liveness after the network becomes idle with leader in commit phase.
#[tokio::test]
async fn timeout_leader_in_commit() {
    let mut util = UTHarness::new_many().await;

    let replica_prepare = util.new_current_replica_prepare(|_| {}).cast().unwrap().msg;
    util.dispatch_replica_prepare_many(
        vec![replica_prepare; util.consensus_threshold()],
        util.keys(),
    )
    .unwrap();

    let base_replica_view = util.replica_view();
    let base_leader_view = util.leader_view();

    util.sim_timeout().await;

    assert_eq!(util.replica_view(), base_replica_view.next());
    assert_eq!(util.replica_phase(), Phase::Prepare);
    assert_eq!(util.leader_view(), base_leader_view);
    // Leader is in `Phase::Commit`, but should still accept prepares from newer views.
    assert_eq!(util.leader_phase(), Phase::Commit);

    util.check_recovery_after_timeout();
}

/// Testing liveness after the network becomes idle with replica in commit phase.
#[tokio::test]
async fn timeout_replica_in_commit() {
    let mut util = UTHarness::new_many().await;

    let leader_prepare = util.new_procedural_leader_prepare_many().await;
    util.dispatch_leader_prepare(leader_prepare).await.unwrap();
    assert_eq!(util.replica_phase(), Phase::Commit);

    let base_replica_view = util.replica_view();
    let base_leader_view = util.leader_view();

    util.sim_timeout().await;

    assert_eq!(util.replica_view(), base_replica_view.next());
    assert_eq!(util.replica_phase(), Phase::Prepare);
    assert_eq!(util.leader_view(), base_leader_view);
    // Leader is in `Phase::Commit`, but should still accept prepares from newer views.
    assert_eq!(util.leader_phase(), Phase::Commit);

    util.check_recovery_after_timeout();
}

/// Testing liveness after the network becomes idle with leader having some cached commit messages for the current view.
#[tokio::test]
async fn timeout_leader_some_commits() {
    let mut util = UTHarness::new_many().await;

    let replica_commit = util.new_procedural_replica_commit_many().await;
    let res = util.dispatch_replica_commit_one(replica_commit);
    assert_matches!(
        res,
        Err(ReplicaCommitError::NumReceivedBelowThreshold {
            num_messages,
            threshold,
        }) => {
            assert_eq!(num_messages, 1);
            assert_eq!(threshold, util.consensus_threshold())
        }
    );

    let base_replica_view = util.replica_view();
    let base_leader_view = util.leader_view();

    util.sim_timeout().await;

    assert_eq!(util.replica_view(), base_replica_view.next());
    assert_eq!(util.replica_phase(), Phase::Prepare);
    assert_eq!(util.leader_view(), base_leader_view);
    // Leader is in `Phase::Commit`, but should still accept prepares from newer views.
    assert_eq!(util.leader_phase(), Phase::Commit);

    util.check_recovery_after_timeout();
}

/// Testing liveness after the network becomes idle with leader in a consecutive prepare phase.
#[tokio::test]
async fn timeout_leader_in_consecutive_prepare() {
    let mut util = UTHarness::new_many().await;

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

    let base_replica_view = util.replica_view();
    let base_leader_view = util.leader_view();

    util.sim_timeout().await;

    assert_eq!(util.replica_view(), base_replica_view.next());
    assert_eq!(util.replica_phase(), Phase::Prepare);
    assert_eq!(util.leader_view(), base_leader_view);
    assert_eq!(util.leader_phase(), Phase::Prepare);

    util.check_recovery_after_timeout();
}
