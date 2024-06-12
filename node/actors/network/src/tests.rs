use crate::testonly;
use tracing::Instrument as _;
use zksync_concurrency::{ctx, scope, testonly::abort_on_panic};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::testonly::TestMemoryStorage;

/// Test that metrics are correctly defined
/// (won't panic during registration).
#[tokio::test]
async fn test_metrics() {
    abort_on_panic();
    let ctx = &mut ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = validator::testonly::Setup::new(rng, 3);
    let cfgs = testonly::new_configs(rng, &setup, 1);
    scope::run!(ctx, |ctx, s| async {
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        let nodes: Vec<_> = cfgs
            .into_iter()
            .enumerate()
            .map(|(i, cfg)| {
                let (node, runner) = testonly::Instance::new(cfg, store.blocks.clone());
                s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
                node
            })
            .collect();
        testonly::instant_network(ctx, nodes.iter()).await?;

        let registry = vise::MetricsCollection::default().collect();
        nodes[0].state().register_metrics();
        let mut encoded_metrics = String::new();
        registry.encode(&mut encoded_metrics, vise::Format::OpenMetricsForPrometheus)?;
        tracing::info!("stats =\n{encoded_metrics}");
        Ok(())
    })
    .await
    .unwrap()
}
