use super::*;
use crate::types::ReplicaState;
use concurrency::ctx;
use rand::{seq::SliceRandom, Rng};
use roles::validator::{Block, BlockNumber, FinalBlock};
use std::iter;
use tempfile::TempDir;

fn init_store<R: Rng>(rng: &mut R) -> (FinalBlock, Storage, TempDir) {
    let genesis_block = FinalBlock {
        block: Block::genesis(vec![]),
        justification: rng.gen(),
    };

    let temp_dir = TempDir::new().unwrap();
    let block_store = Storage::new(&genesis_block, temp_dir.path());

    (genesis_block, block_store, temp_dir)
}

#[test]
fn init_store_twice() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let (genesis_block, block_store, temp_dir) = init_store(rng);
    let block_1 = FinalBlock {
        block: Block {
            parent: genesis_block.block.hash(),
            number: genesis_block.block.number.next(),
            payload: Vec::new(),
        },
        justification: rng.gen(),
    };
    block_store.put_block(&block_1);

    assert_eq!(block_store.get_first_block(), genesis_block);
    assert_eq!(block_store.get_head_block(), block_1);

    drop(block_store);
    let block_store = Storage::new(&genesis_block, temp_dir.path());

    assert_eq!(block_store.get_first_block(), genesis_block);
    assert_eq!(block_store.get_head_block(), block_1);
}

#[test]
fn test_put_block() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let (genesis_block, block_store, _temp_dir) = init_store(rng);

    assert_eq!(block_store.get_first_block(), genesis_block);
    assert_eq!(block_store.get_head_block(), genesis_block);

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
    block_store.put_block(&block_1);

    assert_eq!(block_store.get_first_block(), genesis_block);
    assert_eq!(block_store.get_head_block(), block_1);
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
    block_store.put_block(&block_2);

    assert_eq!(block_store.get_first_block(), genesis_block);
    assert_eq!(block_store.get_head_block(), block_2);
    assert_eq!(*block_subscriber.borrow_and_update(), block_2.block.number);
}

#[test]
fn test_get_missing_block_numbers() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let (genesis_block, block_store, _temp_dir) = init_store(rng);

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
    let mut blocks: Vec<_> = blocks.skip(1).take(100).collect();
    blocks.shuffle(rng);

    assert!(block_store
        .get_missing_block_numbers(BlockNumber(0)..BlockNumber(101))
        .into_iter()
        .map(|number| number.0)
        .eq(1..101));

    for (i, block) in blocks.iter().enumerate() {
        block_store.put_block(block);
        let missing_block_numbers =
            block_store.get_missing_block_numbers(BlockNumber(0)..BlockNumber(101));
        let last_contiguous_block_number = block_store.get_last_contiguous_block_number();

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
