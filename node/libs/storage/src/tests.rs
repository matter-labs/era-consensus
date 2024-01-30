use super::*;
use crate::{testonly::new_store, ReplicaState};
use zksync_concurrency::{ctx, scope, sync, testonly::abort_on_panic};
use zksync_consensus_roles::validator;

#[tokio::test]
async fn test_inmemory_block_store() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let store = &testonly::in_memory::BlockStore::default();
    let mut setup = validator::testonly::GenesisSetup::empty(rng, 3);
    setup.push_blocks(rng, 5);
    let mut want = vec![];
    for block in setup.blocks {
        store.store_next_block(ctx, &block).await.unwrap();
        want.push(block);
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
    let mut genesis = validator::testonly::GenesisSetup::new(rng, 1);
    genesis.push_blocks(rng, 1);

    let (store, runner) = new_store(ctx, &genesis.blocks[0]).await;
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(runner.run(ctx));
        let sub = &mut store.subscribe();
        let state = sub.borrow().clone();
        assert_eq!(state.first, genesis.blocks[0].justification);
        assert_eq!(state.last, genesis.blocks[0].justification);

        store
            .queue_block(ctx, genesis.blocks[1].clone())
            .await
            .unwrap();

        let state = sync::wait_for(ctx, sub, |state| {
            state.last == genesis.blocks[1].justification
        })
        .await?
        .clone();
        assert_eq!(state.first, genesis.blocks[0].justification);
        assert_eq!(state.last, genesis.blocks[1].justification);
        Ok(())
    })
    .await
    .unwrap();
}
