use rand::Rng;
use tracing::Instrument as _;
use zksync_concurrency::{ctx, scope, testonly::abort_on_panic, time};
use zksync_consensus_engine::{testonly::TestEngine, Transaction};
use zksync_consensus_roles::validator;

use crate::testonly;

#[tokio::test(flavor = "multi_thread")]
async fn test_push_tx_propagation() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = validator::testonly::Setup::new(rng, 10);
    let cfgs = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx, s| async {
        let mut nodes = Vec::new();

        for (i, cfg) in cfgs.iter().enumerate() {
            let engine = TestEngine::new(ctx, &setup).await;
            let tx_pool_sender = engine.manager.tx_pool_sender();
            let mempool = engine.im_engine.mempool();
            s.spawn_bg(
                engine
                    .runner
                    .run(ctx)
                    .instrument(tracing::trace_span!("engine", i)),
            );
            let (network, runner) = testonly::Instance::new(cfg.clone(), engine.manager.clone());
            s.spawn_bg(
                runner
                    .run(ctx)
                    .instrument(tracing::trace_span!("network", i)),
            );
            nodes.push((tx_pool_sender, mempool, network));
        }

        ctx.sleep(time::Duration::seconds(1)).await.unwrap();

        let tx: Transaction = rng.gen();
        nodes[0].0.send(tx).unwrap();

        ctx.sleep(time::Duration::seconds(1)).await.unwrap();

        for (i, node) in nodes.iter().enumerate() {
            println!("mempool node[{}]: {:?}", i, node.1.lock().unwrap().len());
        }

        panic!();

        Ok(())
    })
    .await
    .unwrap();
}
