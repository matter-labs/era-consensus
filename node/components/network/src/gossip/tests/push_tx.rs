use rand::Rng;
use tracing::Instrument as _;
use zksync_concurrency::{ctx, scope, testonly::abort_on_panic, time};
use zksync_consensus_engine::{testonly::TestEngine, Transaction};
use zksync_consensus_roles::validator;

use crate::{testonly, CONNECT_RETRY};

#[tokio::test(flavor = "multi_thread")]
async fn test_push_tx_propagation() {
    const NUM_NODES: usize = 10;

    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(40.));
    let rng = &mut ctx.rng();
    let setup = validator::testonly::Setup::new(rng, NUM_NODES);
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

        // Wait enough time to establish connections. Normally at least one retry is
        // necessary to get all connections established.
        ctx.sleep(2 * CONNECT_RETRY).await.unwrap();

        // Pick a random node and add a transaction to its mempool.
        let node_idx = rng.gen_range(0..NUM_NODES);
        let tx: Transaction = rng.gen();
        nodes[node_idx].0.send(tx.clone()).unwrap();

        // Wait for the transaction to be gossiped to all nodes.
        ctx.sleep(time::Duration::seconds(5)).await.unwrap();

        // Check that the transaction is in the mempool of all nodes.
        for node in nodes.iter() {
            let mempool = node.1.lock().unwrap();
            assert_eq!(mempool.len(), 1);
            assert!(mempool.contains(&tx));
        }

        Ok(())
    })
    .await
    .unwrap();
}
