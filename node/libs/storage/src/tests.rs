use super::*;
use crate::{testonly::new_store, ReplicaState};
use zksync_consensus_roles::validator::testonly::Setup;
use zksync_concurrency::{ctx, scope, sync, testonly::abort_on_panic};

#[tokio::test]
async fn test_inmemory_block_store() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new(rng, 3);
    setup.push_blocks(rng, 3);
    setup.fork();
    setup.push_blocks(rng, 3);

    let store = &testonly::in_memory::BlockStore::new(setup.genesis.clone());
    let mut want = vec![];
    for block in &setup.blocks {
        store.store_next_block(ctx, block).await.unwrap();
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
    setup.push_blocks(rng, 1);
    let (store, runner) = new_store(ctx, &setup.genesis).await;
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(runner.run(ctx));
        let sub = &mut store.subscribe();
        let state = sub.borrow().clone();
        assert_eq!(state.first, setup.genesis.forks.root().first_block);
        assert_eq!(state.last, None);

        store
            .queue_block(ctx, setup.blocks[0].clone())
            .await
            .unwrap();

        let state = sync::wait_for(ctx, sub, |state| {
            state.last.as_ref() == Some(&setup.blocks[0].justification)
        })
        .await?
        .clone();
        assert_eq!(state.first, setup.blocks[0].header().number);
        Ok(())
    })
    .await
    .unwrap();
}
