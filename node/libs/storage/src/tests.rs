use super::*;
use crate::types::ReplicaState;
use concurrency::ctx;
use rand::{seq::SliceRandom, Rng};
use roles::validator::{Block, BlockNumber, FinalBlock};
use std::iter;
use tempfile::TempDir;

async fn init_store<R: Rng>(ctx: &ctx::Ctx, rng: &mut R) -> (FinalBlock, RocksdbStorage, TempDir) {
    let genesis_block = FinalBlock {
        block: Block::genesis(vec![]),
        justification: rng.gen(),
    };
    let temp_dir = TempDir::new().unwrap();
    let block_store = RocksdbStorage::new(ctx, &genesis_block, temp_dir.path())
        .await
        .unwrap();
    (genesis_block, block_store, temp_dir)
}

#[tokio::test]
async fn init_store_twice() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let (genesis_block, block_store, temp_dir) = init_store(&ctx, rng).await;
    let block_1 = FinalBlock {
        block: Block {
            parent: genesis_block.block.hash(),
            number: genesis_block.block.number.next(),
            payload: Vec::new(),
        },
        justification: rng.gen(),
    };
    block_store.put_block(&ctx, &block_1).await.unwrap();

    assert_eq!(block_store.first_block(&ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(&ctx).await.unwrap(), block_1);

    drop(block_store);
    let block_store = RocksdbStorage::new(&ctx, &genesis_block, temp_dir.path())
        .await
        .unwrap();

    assert_eq!(block_store.first_block(&ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(&ctx).await.unwrap(), block_1);
}

#[tokio::test]
async fn test_put_block() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let (genesis_block, block_store, _temp_dir) = init_store(&ctx, rng).await;

    assert_eq!(block_store.first_block(&ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(&ctx).await.unwrap(), genesis_block);

    let mut block_subscriber = block_store.subscribe_to_block_writes();
    assert_eq!(*block_subscriber.borrow_and_update(), BlockNumber(0));

    // Test inserting a block with a valid parent.
    let block_1 = FinalBlock {
        block: Block {
            parent: genesis_block.block.hash(),
            number: genesis_block.block.number.next(),
            payload: Vec::new(),
        },
        justification: rng.gen(),
    };
    block_store.put_block(&ctx, &block_1).await.unwrap();

    assert_eq!(block_store.first_block(&ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(&ctx).await.unwrap(), block_1);
    assert_eq!(*block_subscriber.borrow_and_update(), block_1.block.number);

    // Test inserting a block with a valid parent that is not the genesis.
    let block_2 = FinalBlock {
        block: Block {
            parent: block_1.block.hash(),
            number: block_1.block.number.next(),
            payload: Vec::new(),
        },
        justification: rng.gen(),
    };
    block_store.put_block(&ctx, &block_2).await.unwrap();

    assert_eq!(block_store.first_block(&ctx).await.unwrap(), genesis_block);
    assert_eq!(block_store.head_block(&ctx).await.unwrap(), block_2);
    assert_eq!(*block_subscriber.borrow_and_update(), block_2.block.number);
}

fn gen_blocks(rng: &mut impl Rng, genesis_block: FinalBlock, count: usize) -> Vec<FinalBlock> {
    let blocks = iter::successors(Some(genesis_block), |parent| {
        let block = Block {
            parent: parent.block.hash(),
            number: parent.block.number.next(),
            payload: Vec::new(),
        };
        Some(FinalBlock {
            block,
            justification: rng.gen(),
        })
    });
    blocks.skip(1).take(count).collect()
}

#[tokio::test]
async fn test_get_missing_block_numbers() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let (genesis_block, block_store, _temp_dir) = init_store(&ctx, rng).await;
    let mut blocks = gen_blocks(rng, genesis_block, 100);
    blocks.shuffle(rng);

    assert!(block_store
        .missing_block_numbers(&ctx, BlockNumber(0)..BlockNumber(101))
        .await
        .unwrap()
        .into_iter()
        .map(|number| number.0)
        .eq(1..101));

    for (i, block) in blocks.iter().enumerate() {
        block_store.put_block(&ctx, block).await.unwrap();
        let missing_block_numbers = block_store
            .missing_block_numbers(&ctx, BlockNumber(0)..BlockNumber(101))
            .await
            .unwrap();
        let last_contiguous_block_number = block_store
            .last_contiguous_block_number(&ctx)
            .await
            .unwrap();

        let mut expected_block_numbers: Vec<_> =
            blocks[(i + 1)..].iter().map(|b| b.block.number).collect();
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

#[test]
fn test_schema_encode_decode() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let replica = rng.gen::<ReplicaState>();
    assert_eq!(replica, schema::decode(&schema::encode(&replica)).unwrap());
}

#[test]
fn cancellation_is_detected_in_storage_errors() {
    let err = StorageError::from(ctx::Canceled);
    let err = anyhow::Error::from(err);
    assert!(err.root_cause().is::<ctx::Canceled>());
}
