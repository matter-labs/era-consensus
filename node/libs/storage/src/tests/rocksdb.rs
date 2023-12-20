use super::*;
use tempfile::TempDir;

#[async_trait]
impl InitStore for TempDir {
    type Store = RocksdbStorage;

    async fn init_store(&self, ctx: &ctx::Ctx, genesis_block: &validator::FinalBlock) -> Self::Store {
        let db = RocksdbStorage::new(self.path(),10).await.unwrap();
        db.store_next_block(ctx,genesis_block).await.unwrap();
        db
    }
}

#[tokio::test]
async fn initializing_store_twice() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut blocks = vec![testonly::make_genesis_block(rng, validator::ProtocolVersion::EARLIEST)];
    let temp_dir = TempDir::new().unwrap();
    let store = temp_dir.init_store(ctx, &blocks[0]).await;
    blocks.push(testonly::make_block(rng, blocks[0].header(), validator::ProtocolVersion::EARLIEST));
    store.store_next_block(ctx, &blocks[1]).await.unwrap();
    assert_eq!(dump(ctx,&store).await, blocks);
    drop(store);
    let store = temp_dir.init_store(ctx, &blocks[0]).await;
    assert_eq!(dump(ctx,&store).await, blocks);
}

#[tokio::test]
async fn putting_block_for_rocksdb_store() {
    test_put_block(&TempDir::new().unwrap()).await;
}
