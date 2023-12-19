use super::*;
use tempfile::TempDir;

#[async_trait]
impl InitStore for TempDir {
    type Store = RocksdbStorage;

    async fn init_store(&self, ctx: &ctx::Ctx, genesis_block: &FinalBlock) -> Self::Store {
        RocksdbStorage::new(ctx, genesis_block, self.path())
            .await
            .expect("Failed initializing RocksDB")
    }
}

#[tokio::test]
async fn initializing_store_twice() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let genesis_block = testonly::make_genesis_block(rng, ProtocolVersion::EARLIEST);
    let temp_dir = TempDir::new().unwrap();
    let block_store = temp_dir.init_store(ctx, &genesis_block).await;
    let block_1 = testonly::make_block(rng, genesis_block.header(), ProtocolVersion::EARLIEST);
    block_store.put_block(ctx, &block_1).await.unwrap();

    assert_eq!(block_store.first_block(ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(ctx).await.unwrap(), block_1);

    drop(block_store);
    let block_store = temp_dir.init_store(ctx, &genesis_block).await;

    assert_eq!(block_store.first_block(ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(ctx).await.unwrap(), block_1);
}

#[tokio::test]
async fn putting_block_for_rocksdb_store() {
    test_put_block(&TempDir::new().unwrap()).await;
}

#[tokio::test]
async fn getting_missing_block_numbers_for_rocksdb_store() {
    test_get_missing_block_numbers(&TempDir::new().unwrap(), 0).await;
}

#[test_casing(4, [1, 10, 23, 42])]
#[tokio::test]
async fn getting_missing_block_numbers_for_rocksdb_snapshot(skip_count: usize) {
    test_get_missing_block_numbers(&TempDir::new().unwrap(), skip_count).await;
}
