use super::*;
use crate::ReplicaState;
use zksync_concurrency::ctx;

#[tokio::test]
async fn test_inmemory_block_store() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let store = &testonly::in_memory::BlockStore::default();
    let mut want = vec![];
    for block in testonly::random_blocks(ctx).take(5) {
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
