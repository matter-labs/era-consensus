use super::*;
use crate::{testonly::new_store_with_first, ReplicaState};
use zksync_concurrency::{ctx, sync, scope, testonly::abort_on_panic};
use zksync_consensus_roles::validator::testonly::Setup;

#[tokio::test]
async fn test_inmemory_block_store() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new(rng, 3);
    setup.push_blocks(rng, 5);

    let store = &testonly::in_memory::BlockStore::new(
        setup.genesis.clone(),
        setup.genesis.fork.first_block,
    );
    let mut want = vec![];
    for block in &setup.blocks {
        store.queue_next_block(ctx, block.clone()).await.unwrap();
        sync::wait_for(ctx, &mut store.persisted(), |p| p.contains(block.number())).await.unwrap();
        want.push(block.clone());
        assert_eq!(want, testonly::dump(ctx, store).await);
    }
}

#[test]
fn test_schema_encode_decode() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    zksync_protobuf::testonly::test_encode_random::<ReplicaState>(rng);
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
    let (store, runner) = new_store_with_first(ctx, &setup.genesis, first_block.number()).await;
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(runner.run(ctx));
        let want = BlockStoreState {
            first: first_block.number(),
            last: None,
        };

        // Waiting for blocks before genesis first block (or before `state.first_block`) should be ok
        // and should complete immediately.
        for n in [
            setup.genesis.fork.first_block.prev().unwrap(),
            first_block.number().prev().unwrap(),
        ] {
            store.wait_until_queued(ctx, n).await.unwrap();
            store.wait_until_persisted(ctx, n).await.unwrap();
            assert_eq!(want, store.available());
        }

        for block in &setup.blocks {
            store.queue_block(ctx, block.clone()).await.unwrap();
            if block.number() < first_block.number() {
                // Queueing block before first block should be a noop.
                store.wait_until_queued(ctx, block.number()).await.unwrap();
                store
                    .wait_until_persisted(ctx, block.number())
                    .await
                    .unwrap();
                assert_eq!(want, store.available());
            } else {
                // Otherwise the state should be updated as soon as block is queued.
                assert_eq!(
                    BlockStoreState {
                        first: first_block.number(),
                        last: Some(block.justification.clone()),
                    },
                    store.available()
                );
            }
        }
        Ok(())
    })
    .await
    .unwrap();
}
