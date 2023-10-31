//! In-memory storage implementation.

use crate::{
    traits::{BlockStore, ReplicaStateStore, WriteBlockStore},
    types::{MissingBlockNumbers, ReplicaState},
    StorageResult,
};
use async_trait::async_trait;
use concurrency::{
    ctx,
    sync::{self, watch, Mutex},
};
use roles::validator::{BlockNumber, FinalBlock};
use std::{collections::BTreeMap, ops};

#[derive(Debug)]
struct BlocksInMemoryStore {
    blocks: BTreeMap<BlockNumber, FinalBlock>,
    last_contiguous_block_number: BlockNumber,
}

impl BlocksInMemoryStore {
    fn head_block(&self) -> &FinalBlock {
        self.blocks.values().next_back().unwrap()
        // ^ `unwrap()` is safe by construction; the storage contains at least the genesis block
    }

    fn first_block(&self) -> &FinalBlock {
        self.blocks.values().next().unwrap()
        // ^ `unwrap()` is safe by construction; the storage contains at least the genesis block
    }

    fn block(&self, number: BlockNumber) -> Option<&FinalBlock> {
        self.blocks.get(&number)
    }

    fn missing_block_numbers(&self, range: ops::Range<BlockNumber>) -> Vec<BlockNumber> {
        let existing_numbers = self
            .blocks
            .range(range.clone())
            .map(|(&number, _)| Ok(number));
        MissingBlockNumbers::new(range, existing_numbers)
            .map(Result::unwrap)
            .collect()
    }

    fn put_block(&mut self, block: FinalBlock) {
        let block_number = block.header.number;
        tracing::debug!("Inserting block #{block_number} into database");
        if let Some(prev_block) = self.blocks.insert(block_number, block) {
            tracing::debug!(?prev_block, "Block #{block_number} is overwritten");
        } else {
            for (&number, _) in self
                .blocks
                .range(self.last_contiguous_block_number.next()..)
            {
                let expected_block_number = self.last_contiguous_block_number.next();
                if number == expected_block_number {
                    self.last_contiguous_block_number = expected_block_number;
                } else {
                    return;
                }
            }
        }
    }
}

/// In-memory store.
#[derive(Debug)]
pub struct InMemoryStorage {
    blocks: Mutex<BlocksInMemoryStore>,
    replica_state: Mutex<Option<ReplicaState>>,
    blocks_sender: watch::Sender<BlockNumber>,
}

impl InMemoryStorage {
    /// Creates a new store containing only the specified `genesis_block`.
    pub fn new(genesis_block: FinalBlock) -> Self {
        let genesis_block_number = genesis_block.header.number;
        Self {
            blocks: Mutex::new(BlocksInMemoryStore {
                blocks: BTreeMap::from([(genesis_block_number, genesis_block)]),
                last_contiguous_block_number: genesis_block_number,
            }),
            replica_state: Mutex::default(),
            blocks_sender: watch::channel(genesis_block_number).0,
        }
    }
}

#[async_trait]
impl BlockStore for InMemoryStorage {
    async fn head_block(&self, ctx: &ctx::Ctx) -> StorageResult<FinalBlock> {
        Ok(sync::lock(ctx, &self.blocks).await?.head_block().clone())
    }

    async fn first_block(&self, ctx: &ctx::Ctx) -> StorageResult<FinalBlock> {
        Ok(sync::lock(ctx, &self.blocks).await?.first_block().clone())
    }

    async fn last_contiguous_block_number(&self, ctx: &ctx::Ctx) -> StorageResult<BlockNumber> {
        Ok(sync::lock(ctx, &self.blocks)
            .await?
            .last_contiguous_block_number)
    }

    async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: BlockNumber,
    ) -> StorageResult<Option<FinalBlock>> {
        Ok(sync::lock(ctx, &self.blocks).await?.block(number).cloned())
    }

    async fn missing_block_numbers(
        &self,
        ctx: &ctx::Ctx,
        range: ops::Range<BlockNumber>,
    ) -> StorageResult<Vec<BlockNumber>> {
        Ok(sync::lock(ctx, &self.blocks)
            .await?
            .missing_block_numbers(range))
    }

    fn subscribe_to_block_writes(&self) -> watch::Receiver<BlockNumber> {
        self.blocks_sender.subscribe()
    }
}

#[async_trait]
impl WriteBlockStore for InMemoryStorage {
    async fn put_block(&self, ctx: &ctx::Ctx, block: &FinalBlock) -> StorageResult<()> {
        sync::lock(ctx, &self.blocks)
            .await?
            .put_block(block.clone());
        self.blocks_sender.send_replace(block.header.number);
        Ok(())
    }
}

#[async_trait]
impl ReplicaStateStore for InMemoryStorage {
    async fn replica_state(&self, ctx: &ctx::Ctx) -> StorageResult<Option<ReplicaState>> {
        Ok(sync::lock(ctx, &self.replica_state).await?.clone())
    }

    async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> StorageResult<()> {
        *sync::lock(ctx, &self.replica_state).await? = Some(replica_state.clone());
        Ok(())
    }
}
