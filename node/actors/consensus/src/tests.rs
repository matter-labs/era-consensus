use crate::{
    misc::consensus_threshold,
    testonly::{Behavior, Network, Test},
};
use zksync_concurrency::ctx;

async fn run_test(behavior: Behavior, network: Network) {
    concurrency::testonly::abort_on_panic();
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
