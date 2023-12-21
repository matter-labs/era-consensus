use super::*;
use crate::{PersistentBlockStore,ReplicaState};
use async_trait::async_trait;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator::{self};
use rand::Rng;

#[cfg(feature = "rocksdb")]
mod rocksdb;

#[async_trait]
trait InitStore {
    type Store: PersistentBlockStore;
    async fn init_store(&self, ctx: &ctx::Ctx, genesis_block: &validator::FinalBlock) -> Self::Store;
}

#[async_trait]
impl InitStore for () {
    type Store = testonly::in_memory::BlockStore;

    async fn init_store(&self, _ctx: &ctx::Ctx, genesis_block: &validator::FinalBlock) -> Self::Store {
        Self::Store::new(genesis_block.clone())
    }
}

fn make_genesis(rng: &mut impl Rng) -> validator::FinalBlock {
    validator::testonly::make_genesis_block(rng, validator::ProtocolVersion::EARLIEST)
}

fn make_block(rng: &mut impl Rng, parent: &validator::BlockHeader) -> validator::FinalBlock {
    validator::testonly::make_block(rng,parent,validator::ProtocolVersion::EARLIEST) 
}

async fn dump(ctx: &ctx::Ctx, store: &dyn PersistentBlockStore) -> Vec<validator::FinalBlock> {
    let mut blocks = vec![];
    let range = store.available_blocks(ctx).await.unwrap();
    for n in range.start.0..range.end.0 {
        let n = validator::BlockNumber(n);
        let block = store.block(ctx,n).await.unwrap();
        assert_eq!(block.header().number, n);
        blocks.push(block);
    }
    assert!(store.block(ctx,range.end).await.is_err());
    blocks
}

async fn test_put_block(store_factory: &impl InitStore) {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut blocks = vec![make_genesis(rng)];
    let store = &store_factory.init_store(ctx, &blocks[0]).await;
    assert_eq!(dump(ctx,store).await,blocks);

    // Test inserting a block with a valid parent.
    blocks.push(make_block(rng, blocks[0].header()));
    store.store_next_block(ctx, &blocks[1]).await.unwrap();
    assert_eq!(dump(ctx,store).await,blocks);

    // Test inserting a block with a valid parent that is not the genesis.
    blocks.push(make_block(rng, blocks[1].header()));
    store.store_next_block(ctx, &blocks[2]).await.unwrap();
    assert_eq!(dump(ctx,store).await,blocks);
}

#[tokio::test]
async fn putting_block_for_in_memory_store() {
    test_put_block(&()).await;
}

#[test]
fn test_schema_encode_decode() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    zksync_protobuf::testonly::test_encode_random::<_,ReplicaState>(rng);
}
