use super::*;
use crate::types::ReplicaState;
use async_trait::async_trait;
use rand::{seq::SliceRandom, Rng};
use std::iter;
use test_casing::test_casing;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator::{testonly, BlockNumber, FinalBlock, ProtocolVersion};

#[cfg(feature = "rocksdb")]
mod rocksdb;

#[async_trait]
trait InitStore {
    type Store: WriteBlockStore + ReplicaStateStore;

    async fn init_store(&self, ctx: &ctx::Ctx, genesis_block: &FinalBlock) -> Self::Store;
}

#[async_trait]
impl InitStore for () {
    type Store = InMemoryStorage;

    async fn init_store(&self, _ctx: &ctx::Ctx, genesis_block: &FinalBlock) -> Self::Store {
        InMemoryStorage::new(genesis_block.clone())
    }
}

fn gen_blocks(rng: &mut impl Rng, genesis_block: FinalBlock, count: usize) -> Vec<FinalBlock> {
    let blocks = iter::successors(Some(genesis_block), |parent| {
        Some(testonly::make_block(
            rng,
            parent.header(),
            ProtocolVersion::EARLIEST,
        ))
    });
    blocks.skip(1).take(count).collect()
}

async fn test_put_block(store_factory: &impl InitStore) {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let genesis_block = testonly::make_genesis_block(rng, ProtocolVersion::EARLIEST);
    let block_store = store_factory.init_store(ctx, &genesis_block).await;

    assert_eq!(block_store.first_block(ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(ctx).await.unwrap(), genesis_block);

    let mut block_subscriber = block_store.subscribe_to_block_writes();
    assert_eq!(*block_subscriber.borrow_and_update(), BlockNumber(0));

    // Test inserting a block with a valid parent.
    let block_1 = testonly::make_block(rng, genesis_block.header(), ProtocolVersion::EARLIEST);
    block_store.put_block(ctx, &block_1).await.unwrap();

    assert_eq!(block_store.first_block(ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(ctx).await.unwrap(), block_1);
    assert_eq!(
        *block_subscriber.borrow_and_update(),
        block_1.header().number
    );

    // Test inserting a block with a valid parent that is not the genesis.
    let block_2 = testonly::make_block(rng, block_1.header(), ProtocolVersion::EARLIEST);
    block_store.put_block(ctx, &block_2).await.unwrap();

    assert_eq!(block_store.first_block(ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(ctx).await.unwrap(), block_2);
    assert_eq!(
        *block_subscriber.borrow_and_update(),
        block_2.header().number
    );
}

#[tokio::test]
async fn putting_block_for_in_memory_store() {
    test_put_block(&()).await;
}

async fn test_get_missing_block_numbers(store_factory: &impl InitStore, skip_count: usize) {
    assert!(skip_count < 100);

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut genesis_block = testonly::make_genesis_block(rng, ProtocolVersion::EARLIEST);
    let mut blocks = gen_blocks(rng, genesis_block.clone(), 100);
    if skip_count > 0 {
        genesis_block = blocks[skip_count - 1].clone();
        blocks = blocks[skip_count..].to_vec();
    }
    let block_range = BlockNumber(skip_count as u64)..BlockNumber(101);

    let block_store = store_factory.init_store(ctx, &genesis_block).await;
    blocks.shuffle(rng);

    assert!(block_store
        .missing_block_numbers(ctx, block_range.clone())
        .await
        .unwrap()
        .into_iter()
        .map(|number| number.0)
        .eq(skip_count as u64 + 1..101));

    for (i, block) in blocks.iter().enumerate() {
        block_store.put_block(ctx, block).await.unwrap();
        let missing_block_numbers = block_store
            .missing_block_numbers(ctx, block_range.clone())
            .await
            .unwrap();
        let last_contiguous_block_number =
            block_store.last_contiguous_block_number(ctx).await.unwrap();

        let mut expected_block_numbers: Vec<_> = blocks[(i + 1)..]
            .iter()
            .map(|b| b.header().number)
            .collect();
        expected_block_numbers.sort_unstable();

        assert_eq!(missing_block_numbers, expected_block_numbers);
        if let Some(&first_missing_block_number) = expected_block_numbers.first() {
            assert_eq!(
                last_contiguous_block_number.next(),
                first_missing_block_number
            );
        } else {
            assert_eq!(last_contiguous_block_number, BlockNumber(100));
        }
    }
}

#[tokio::test]
async fn getting_missing_block_numbers_for_in_memory_store() {
    test_get_missing_block_numbers(&(), 0).await;
}

#[test_casing(4, [1, 10, 23, 42])]
#[tokio::test]
async fn getting_missing_block_numbers_for_snapshot(skip_count: usize) {
    test_get_missing_block_numbers(&(), skip_count).await;
}

#[test]
fn test_schema_encode_decode() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let replica = rng.gen::<ReplicaState>();
    assert_eq!(
        replica,
        zksync_protobuf::decode(&zksync_protobuf::encode(&replica)).unwrap()
    );
}
