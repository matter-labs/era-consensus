use crate::{run_network, testonly};
use zksync_concurrency::{ctx, scope};
use tracing::Instrument as _;
use zksync_consensus_utils::pipe;

/// Test that metrics are correctly defined
/// (won't panic during registration).
#[tokio::test]
async fn test_metrics() {
    concurrency::testonly::abort_on_panic();
    let ctx = &mut ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let nodes = testonly::Instance::new(rng, 3, 1);
    scope::run!(ctx, |ctx, s| async {
        for (i, n) in nodes.iter().enumerate() {
            let (network_pipe, _) = pipe::new();
            s.spawn_bg(
                run_network(ctx, n.state.clone(), network_pipe)
                    .instrument(tracing::info_span!("node", i)),
            );
        }
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
