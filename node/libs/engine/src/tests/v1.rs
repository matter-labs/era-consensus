use zksync_concurrency::{ctx, scope, sync, testonly::abort_on_panic};
use zksync_consensus_roles::validator::testonly::Setup;

use crate::{
    block_store,
    testonly::{self, TestEngineManager},
    BlockStoreState, PersistentBlockStore as _,
};

#[tokio::test]
async fn test_inmemory_block_store_v1() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new(rng, 3);
    setup.push_blocks_v1(rng, 5);

    let store = &testonly::in_memory::Engine::new(
        &setup,
        setup.blocks.first().as_ref().unwrap().number(),
    );
    let mut want = vec![];
    for block in &setup.blocks {
        tracing::info!("block = {:?}", block.number());
        store.queue_next_block(ctx, block.clone()).await.unwrap();
        sync::wait_for(ctx, &mut store.persisted(), |p| p.contains(block.number()))
            .await
            .unwrap();
        want.push(block.clone());
        assert_eq!(want, testonly::dump(ctx, store).await);
    }
}

#[tokio::test]
async fn test_get_not_cached_block_v1() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new_without_pregenesis(rng, 1);
    setup.push_blocks_v1(rng, block_store::CACHE_CAPACITY + 5);

    scope::run!(ctx, |ctx, s| async {
        let store = TestEngineManager::new(ctx, &setup).await;
        s.spawn_bg(store.runner.run(ctx));
        // Persist more blocks than the cache size.
        for block in &setup.blocks {
            store.blocks.queue_block(ctx, block.clone()).await.unwrap();
        }
        store
            .blocks
            .wait_until_persisted(ctx, setup.blocks.last().as_ref().unwrap().number())
            .await
            .unwrap();
        // Request the first block (not in cache).
        assert_eq!(
            setup.blocks[0],
            store
                .blocks
                .block(ctx, setup.blocks[0].number())
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
async fn test_state_updates_v1() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new(rng, 1);
    setup.push_blocks_v1(rng, 5);

    // Create store with non-trivial first block.
    let first_block = &setup.blocks[2];
    let store =
        TestEngineManager::new_store_with_first_block(ctx, &setup, first_block.number()).await;
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(store.runner.run(ctx));
        let want = BlockStoreState {
            first: first_block.number(),
            last: None,
        };

        for block in &setup.blocks {
            store.blocks.queue_block(ctx, block.clone()).await.unwrap();
            if block.number() < first_block.number() {
                // Queueing block before first block should be a noop.
                store
                    .blocks
                    .wait_until_queued(ctx, block.number())
                    .await
                    .unwrap();
                store
                    .blocks
                    .wait_until_persisted(ctx, block.number())
                    .await
                    .unwrap();
                assert_eq!(want, store.blocks.queued());
            } else {
                // Otherwise the state should be updated as soon as block is queued.
                assert_eq!(
                    BlockStoreState {
                        first: first_block.number(),
                        last: Some(block.into()),
                    },
                    store.blocks.queued()
                );
            }
        }
        Ok(())
    })
    .await
    .unwrap();
}
