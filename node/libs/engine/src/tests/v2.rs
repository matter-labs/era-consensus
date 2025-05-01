use zksync_concurrency::{ctx, scope, sync, testonly::abort_on_panic};
use zksync_consensus_roles::validator::testonly::Setup;

use crate::{
    block_store::BlockStore,
    testonly::{self, TestEngine},
    BlockStoreState, EngineInterface as _,
};

#[tokio::test]
async fn test_inmemory_block_store_v2() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let mut setup = Setup::new(rng, 3);
    setup.push_blocks_v2(rng, 5);
    let engine = &testonly::in_memory::Engine::new_random(
        &setup,
        setup.blocks.first().as_ref().unwrap().number(),
    );

    let mut want = vec![];
    for block in &setup.blocks {
        tracing::info!("block = {:?}", block.number());
        engine.queue_next_block(ctx, block.clone()).await.unwrap();
        sync::wait_for(ctx, &mut engine.persisted(), |p| p.contains(block.number()))
            .await
            .unwrap();
        want.push(block.clone());
        assert_eq!(want, testonly::dump(ctx, engine).await);
    }
}

#[tokio::test]
async fn test_get_not_cached_block_v2() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let mut setup = Setup::new_without_pregenesis(rng, 1);
    setup.push_blocks_v2(rng, BlockStore::CACHE_CAPACITY + 5);

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));

        // Persist more blocks than the cache size.
        for block in &setup.blocks {
            engine
                .manager
                .queue_block(
                    ctx,
                    block.clone(),
                    setup.genesis.validators_schedule.as_ref(),
                )
                .await
                .unwrap();
        }
        engine
            .manager
            .wait_until_persisted(ctx, setup.blocks.last().as_ref().unwrap().number())
            .await
            .unwrap();

        // Request the first block (not in cache).
        assert_eq!(
            setup.blocks[0],
            engine
                .manager
                .get_block(ctx, setup.blocks[0].number())
                .await
                .unwrap()
                .unwrap()
        );

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_state_updates_v2() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let mut setup = Setup::new(rng, 1);
    setup.push_blocks_v2(rng, 5);

    // Create store with non-trivial first block.
    let first_block = &setup.blocks[2];
    let engine = TestEngine::new_with_first_block(ctx, &setup, first_block.number()).await;

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(engine.runner.run(ctx));
        let want = BlockStoreState {
            first: first_block.number(),
            last: None,
        };

        for block in &setup.blocks {
            engine
                .manager
                .queue_block(
                    ctx,
                    block.clone(),
                    setup.genesis.validators_schedule.as_ref(),
                )
                .await
                .unwrap();
            if block.number() < first_block.number() {
                // Queueing block before first block should be a noop.
                engine
                    .manager
                    .wait_until_queued(ctx, block.number())
                    .await
                    .unwrap();
                engine
                    .manager
                    .wait_until_persisted(ctx, block.number())
                    .await
                    .unwrap();
                assert_eq!(want, engine.manager.queued());
            } else {
                // Otherwise the state should be updated as soon as block is queued.
                assert_eq!(
                    BlockStoreState {
                        first: first_block.number(),
                        last: Some(block.into()),
                    },
                    engine.manager.queued()
                );
            }
        }
        Ok(())
    })
    .await
    .unwrap();
}
