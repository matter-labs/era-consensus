use crate::{testonly};
use tracing::Instrument as _;
use zksync_concurrency::{ctx, scope, testonly::abort_on_panic};
use zksync_consensus_roles::validator;

/// Test that metrics are correctly defined
/// (won't panic during registration).
#[tokio::test]
async fn test_metrics() {
    abort_on_panic();
    let ctx = &mut ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = validator::testonly::GenesisSetup::new(rng, 3);
    let cfgs = testonly::new_configs(rng, &setup, 1);
    scope::run!(ctx, |ctx, s| async {
        let (store,runner) = testonly::new_store(ctx,&setup.blocks[0]).await;
        s.spawn_bg(runner.run(ctx));
        let nodes : Vec<_> = cfgs.into_iter().enumerate().map(|(i,cfg)| {
            let (node,runner) = testonly::Instance::new(cfg, store.clone());
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            node
        }).collect();
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
