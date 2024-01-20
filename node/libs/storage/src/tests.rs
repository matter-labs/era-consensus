use super::*;
use crate::ReplicaState;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;

#[tokio::test]
async fn test_inmemory_block_store() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let store = &testonly::in_memory::BlockStore::default();
    let mut setup = validator::testonly::GenesisSetup::empty(rng,3);
    setup.push_blocks(rng,5);
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
    zksync_protobuf::testonly::test_encode_random::<_, ReplicaState>(rng);
}

// TODO: test moved from sync_blocks
#[tokio::test]
async fn subscribing_to_state_updates() {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let protocol_version = validator::ProtocolVersion::EARLIEST;
    let genesis_block = make_genesis_block(rng, protocol_version);
    let block_1 = make_block(rng, genesis_block.header(), protocol_version);

    let (storage, runner) = make_store(ctx, genesis_block.clone()).await;
    let (actor_pipe, _dispatcher_pipe) = pipe::new();
    let mut state_subscriber = storage.subscribe();

    let cfg: Config = rng.gen();
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(cfg.run(ctx, actor_pipe, storage.clone()));
        s.spawn_bg(async {
            assert!(ctx.sleep(TEST_TIMEOUT).await.is_err(), "Test timed out");
            anyhow::Ok(())
        });

        let state = state_subscriber.borrow().clone();
        assert_eq!(state.first, genesis_block.justification);
        assert_eq!(state.last, genesis_block.justification);
        storage.queue_block(ctx, block_1.clone()).await.unwrap();

        let state = sync::wait_for(ctx, &mut state_subscriber, |state| {
            state.next() > block_1.header().number
        })
        .await
        .unwrap()
        .clone();
        assert_eq!(state.first, genesis_block.justification);
        assert_eq!(state.last, block_1.justification);
        Ok(())
    })
    .await
    .unwrap();
}
