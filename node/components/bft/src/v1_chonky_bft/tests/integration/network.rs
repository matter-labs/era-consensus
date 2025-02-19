use crate::{
    v1_chonky_bft::testonly::{IntegrationTestConfig, TestNetwork},
    testonly::Behavior,
};
use zksync_concurrency::{ctx, time};
use zksync_consensus_roles::validator;

async fn run_test(behavior: Behavior, network: TestNetwork) {
    tokio::time::pause();
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(60));
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);

    const NODES: usize = 11;
    let mut nodes = vec![(behavior, 1u64); NODES];
    // validator::threshold(NODES) will calculate required nodes to validate a message
    // given each node weight is 1
    let honest_nodes_amount = validator::quorum_threshold(NODES as u64) as usize;
    for n in &mut nodes[0..honest_nodes_amount] {
        n.0 = Behavior::Honest;
    }
    IntegrationTestConfig {
        network,
        nodes,
        blocks_to_finalize: 10,
    }
    .run(ctx)
    .await
    .unwrap()
}

#[tokio::test]
async fn honest_real_network() {
    run_test(Behavior::Honest, TestNetwork::Real).await
}

#[tokio::test]
async fn offline_real_network() {
    run_test(Behavior::Offline, TestNetwork::Real).await
}

#[tokio::test]
async fn honest_not_proposing_real_network() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(5.));
    IntegrationTestConfig {
        network: TestNetwork::Real,
        nodes: vec![(Behavior::Honest, 1), (Behavior::HonestNotProposing, 1)],
        blocks_to_finalize: 10,
    }
    .run(ctx)
    .await
    .unwrap()
}
