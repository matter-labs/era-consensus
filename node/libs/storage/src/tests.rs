use super::*;
use crate::types::ReplicaState;
use async_trait::async_trait;
use concurrency::{
    ctx::{self, channel},
    sync,
    sync::watch,
    time,
};
use rand::{rngs::StdRng, seq::SliceRandom, Rng};
use roles::validator::{Block, BlockNumber, FinalBlock};
use std::{iter, ops};
use tempfile::TempDir;
use test_casing::test_casing;

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

#[derive(Debug)]
struct MockContiguousStore {
    inner: RocksdbStorage,
    block_sender: channel::Sender<FinalBlock>,
}

impl MockContiguousStore {
    fn new(inner: RocksdbStorage) -> (Self, channel::Receiver<FinalBlock>) {
        let (block_sender, block_receiver) = channel::bounded(1);
        let this = Self {
            inner,
            block_sender,
        };
        (this, block_receiver)
    }

    async fn run_updates(
        &self,
        ctx: &ctx::Ctx,
        mut block_receiver: channel::Receiver<FinalBlock>,
    ) -> StorageResult<()> {
        let rng = &mut ctx.rng();
        while let Ok(block) = block_receiver.recv(ctx).await {
            let sleep_duration = time::Duration::milliseconds(rng.gen_range(0..5));
            ctx.sleep(sleep_duration).await?;
            self.inner.put_block(ctx, &block).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockStore for MockContiguousStore {
    async fn head_block(&self, ctx: &ctx::Ctx) -> StorageResult<FinalBlock> {
        self.inner.head_block(ctx).await
    }

    async fn first_block(&self, ctx: &ctx::Ctx) -> StorageResult<FinalBlock> {
        self.inner.first_block(ctx).await
    }

    async fn last_contiguous_block_number(&self, ctx: &ctx::Ctx) -> StorageResult<BlockNumber> {
        self.inner.last_contiguous_block_number(ctx).await
    }

    async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: BlockNumber,
    ) -> StorageResult<Option<FinalBlock>> {
        self.inner.block(ctx, number).await
    }

    async fn missing_block_numbers(
        &self,
        ctx: &ctx::Ctx,
        range: ops::Range<BlockNumber>,
    ) -> StorageResult<Vec<BlockNumber>> {
        self.inner.missing_block_numbers(ctx, range).await
    }

    fn subscribe_to_block_writes(&self) -> watch::Receiver<BlockNumber> {
        self.inner.subscribe_to_block_writes()
    }
}

#[async_trait]
impl ContiguousBlockStore for MockContiguousStore {
    async fn schedule_next_block(&self, ctx: &ctx::Ctx, block: &FinalBlock) -> StorageResult<()> {
        let head_block_number = self.head_block(ctx).await?.block.number;
        assert_eq!(block.block.number, head_block_number.next());
        self.block_sender
            .try_send(block.clone())
            .expect("BufferedStorage is rushing");
        Ok(())
    }
}

#[tracing::instrument(level = "trace", skip(shuffle_blocks))]
async fn test_buffered_storage(
    block_count: usize,
    block_interval: time::Duration,
    shuffle_blocks: impl FnOnce(&mut StdRng, &mut [FinalBlock]),
) {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let (genesis_block, block_store, _temp_dir) = init_store(ctx, rng).await;
    let (block_store, block_receiver) = MockContiguousStore::new(block_store);
    let buffered_store = BufferedStorage::new(block_store);

    // Check initial values returned by the store.
    assert_eq!(buffered_store.head_block(ctx).await.unwrap(), genesis_block);
    assert_eq!(
        buffered_store
            .block(ctx, BlockNumber(0))
            .await
            .unwrap()
            .as_ref(),
        Some(&genesis_block)
    );
    assert_eq!(
        buffered_store.block(ctx, BlockNumber(1)).await.unwrap(),
        None
    );
    let mut subscriber = buffered_store.subscribe_to_block_writes();
    assert_eq!(*subscriber.borrow(), BlockNumber(0));

    let mut blocks = gen_blocks(rng, genesis_block.clone(), block_count);
    shuffle_blocks(rng, &mut blocks);
    let last_block_number = BlockNumber(block_count as u64);

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(buffered_store.as_ref().run_updates(ctx, block_receiver));
        s.spawn_bg(async {
            let err = buffered_store.listen_to_updates(ctx).await.unwrap_err();
            match &err {
                StorageError::Canceled(_) => Ok(()), // Test has successfully finished
                StorageError::Database(_) => Err(err),
            }
        });

        for (idx, block) in blocks.iter().enumerate() {
            buffered_store.put_block(ctx, block).await?;
            let new_block_number = *sync::changed(ctx, &mut subscriber).await?;
            assert_eq!(new_block_number, block.block.number);

            // Check that all written blocks are immediately accessible.
            for existing_block in &blocks[0..=idx] {
                let number = existing_block.block.number;
                assert_eq!(
                    buffered_store.block(ctx, number).await?.as_ref(),
                    Some(existing_block)
                );
            }
            assert_eq!(buffered_store.first_block(ctx).await?, genesis_block);

            let expected_head_block = blocks[0..=idx]
                .iter()
                .max_by_key(|block| block.block.number)
                .unwrap();
            assert_eq!(buffered_store.head_block(ctx).await?, *expected_head_block);

            let expected_last_contiguous_block = blocks[(idx + 1)..]
                .iter()
                .map(|block| block.block.number)
                .min()
                .map_or(last_block_number, BlockNumber::prev);
            assert_eq!(
                buffered_store.last_contiguous_block_number(ctx).await?,
                expected_last_contiguous_block
            );

            ctx.sleep(block_interval).await?;
        }

        let mut inner_subscriber = buffered_store.as_ref().subscribe_to_block_writes();
        while buffered_store
            .as_ref()
            .last_contiguous_block_number(ctx)
            .await?
            < last_block_number
        {
            sync::changed(ctx, &mut inner_subscriber).await?;
        }

        assert_eq!(buffered_store.buffer_len().await, 0);
        Ok(())
    })
    .await
    .unwrap();
}

// Choose intervals so that they are both smaller and larger than the sleep duration in
// `MockContiguousStore::run_updates()`.
const BLOCK_INTERVALS: [time::Duration; 4] = [
    time::Duration::ZERO,
    time::Duration::milliseconds(3),
    time::Duration::milliseconds(5),
    time::Duration::milliseconds(10),
];

#[test_casing(4, BLOCK_INTERVALS)]
#[tokio::test]
async fn buffered_storage_with_sequential_blocks(block_interval: time::Duration) {
    test_buffered_storage(30, block_interval, |_, _| {
        // Do not perform shuffling
    })
    .await;
}

#[test_casing(4, BLOCK_INTERVALS)]
#[tokio::test]
async fn buffered_storage_with_random_blocks(block_interval: time::Duration) {
    test_buffered_storage(30, block_interval, |rng, blocks| blocks.shuffle(rng)).await;
}

#[test_casing(4, BLOCK_INTERVALS)]
#[tokio::test]
async fn buffered_storage_with_slightly_shuffled_blocks(block_interval: time::Duration) {
    test_buffered_storage(30, block_interval, |rng, blocks| {
        for chunk in blocks.chunks_mut(4) {
            chunk.shuffle(rng);
        }
    })
    .await;
}
