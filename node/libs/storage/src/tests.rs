use super::*;
use crate::{testonly::TestMemoryStorage, ReplicaState};
use rand::Rng as _;
use zksync_concurrency::{ctx, scope, sync, testonly::abort_on_panic};
use zksync_consensus_roles::{
    validator,
    validator::testonly::{Setup, SetupSpec},
};

#[tokio::test]
async fn test_inmemory_block_store() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new(rng, 3);
    setup.push_blocks(rng, 5);

    let store = &testonly::in_memory::BlockStore::new(
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

// Test checking that store doesn't accept pre-genesis blocks with invalid justification.
#[tokio::test]
async fn test_invalid_justification() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut spec = SetupSpec::new(rng, 1);
    spec.first_block = spec.first_pregenesis_block + 2;
    let setup = Setup::from_spec(rng, spec);
    scope::run!(ctx, |ctx, s| async {
        let store = TestMemoryStorage::new(ctx, &setup).await;
        s.spawn_bg(store.runner.run(ctx));
        let store = store.blocks;
        // Insert a correct block first.
        store
            .queue_block(ctx, setup.blocks[0].clone())
            .await
            .unwrap();
        // Insert an incorrect second block.
        let validator::Block::PreGenesis(mut b) = setup.blocks[1].clone() else {
            panic!()
        };
        b.justification = rng.gen();
        store.queue_block(ctx, b.into()).await.unwrap_err();
        Ok(())
    })
    .await
    .unwrap();
}

#[test]
fn test_schema_encode_decode() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    zksync_protobuf::testonly::test_encode_random::<ReplicaState>(rng);
}

#[tokio::test]
async fn test_get_not_cached_block() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new(rng, 1);
    setup.push_blocks(rng, block_store::CACHE_CAPACITY + 5);
    scope::run!(ctx, |ctx, s| async {
        let store = TestMemoryStorage::new(ctx, &setup).await;
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
async fn test_state_updates() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new(rng, 1);
    setup.push_blocks(rng, 5);
    // Create store with non-trivial first block.
    let first_block = &setup.blocks[2];
    let store =
        TestMemoryStorage::new_store_with_first_block(ctx, &setup, first_block.number()).await;
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(store.runner.run(ctx));
        let want = BlockStoreState {
            first: first_block.number(),
            last: None,
        };

        // Waiting for blocks before genesis first block (or before `state.first_block`) should be ok
        // and should complete immediately.
        for n in [
            setup.genesis.first_block.prev().unwrap(),
            first_block.number().prev().unwrap(),
        ] {
            store.blocks.wait_until_queued(ctx, n).await.unwrap();
            store.blocks.wait_until_persisted(ctx, n).await.unwrap();
            assert_eq!(want, store.blocks.queued());
        }

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
