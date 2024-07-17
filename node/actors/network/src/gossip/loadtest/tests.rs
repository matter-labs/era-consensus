use super::*;
use crate::testonly;
use zksync_concurrency::{ctx, scope, sync, testonly::abort_on_panic};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::testonly::TestMemoryStorage;

#[tokio::test]
async fn test_loadtest() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let mut setup = validator::testonly::Setup::new(rng, 1);
    setup.push_blocks(rng, 10);
    let mut cfg = testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.gossip.dynamic_inbound_limit = 7;

    scope::run!(ctx, |ctx, s| async {
        // Spawn the node.
        let stores = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(stores.runner.run(ctx));
        let (node, runner) =
            testonly::Instance::new(cfg.clone(), stores.blocks.clone(), stores.batches.clone());
        s.spawn_bg(runner.run(ctx));

        // Fill the storage with some blocks.
        for b in &setup.blocks {
            stores
                .blocks
                .queue_block(ctx, b.clone())
                .await
                .context("queue_block()")?;
        }

        let (send, recv) = ctx::channel::bounded(10);

        // Run the loadtest.
        s.spawn_bg(async {
            Loadtest {
                addr: cfg.public_addr.clone(),
                peer: cfg.gossip.key.public(),
                genesis: setup.genesis.clone(),
                traffic_pattern: TrafficPattern::Random,
                output: Some(send),
            }
            .run(ctx)
            .await?;
            Ok(())
        });

        s.spawn(async {
            // Wait for a bunch of blocks to be received.
            let mut recv = recv;
            let mut count = 0;
            while count < 100 {
                // Count only responses with actual blocks.
                if recv.recv(ctx).await?.is_some() {
                    count += 1;
                }
            }
            Ok(())
        });
        s.spawn(async {
            // Wait for the inbound connections to get saturated.
            let node = node;
            let sub = &mut node.net.gossip.inbound.subscribe();
            sync::wait_for(ctx, sub, |pool| {
                pool.current().len() == cfg.gossip.dynamic_inbound_limit
            })
            .await?;
            Ok(())
        });
        Ok(())
    })
    .await
    .unwrap();
}
