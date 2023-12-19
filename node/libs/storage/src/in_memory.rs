//! In-memory storage implementation.

use crate::{
    traits::{BlockStore, ReplicaStateStore, WriteBlockStore},
    types::{MissingBlockNumbers, ReplicaState},
};
use async_trait::async_trait;
use std::{collections::BTreeMap, ops};
use zksync_concurrency::{
    ctx,
    sync::{watch, Mutex},
};
use zksync_consensus_roles::validator;

#[derive(Debug)]
struct BlocksInMemoryStore {
    blocks: BTreeMap<validator::BlockNumber, validator::FinalBlock>,
    last_contiguous_block_number: validator::BlockNumber,
}

impl BlocksInMemoryStore {
    fn head_block(&self) -> &validator::FinalBlock {
        self.blocks.values().next_back().unwrap()
        // ^ `unwrap()` is safe by construction; the storage contains at least the genesis block
    }

    fn first_block(&self) -> &validator::FinalBlock {
        self.blocks.values().next().unwrap()
        // ^ `unwrap()` is safe by construction; the storage contains at least the genesis block
    }

    fn block(&self, number: validator::BlockNumber) -> Option<&validator::FinalBlock> {
        self.blocks.get(&number)
    }

    fn missing_block_numbers(
        &self,
        range: ops::Range<validator::BlockNumber>,
    ) -> Vec<validator::BlockNumber> {
        let existing_numbers = self
            .blocks
            .range(range.clone())
            .map(|(&number, _)| Ok(number));
        MissingBlockNumbers::new(range, existing_numbers)
            .map(Result::unwrap)
            .collect()
    }

    fn put_block(&mut self, block: validator::FinalBlock) {
        let block_number = block.header().number;
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
    blocks_sender: watch::Sender<validator::BlockNumber>,
}

impl InMemoryStorage {
    /// Creates a new store containing only the specified `genesis_block`.
    pub fn new(genesis_block: validator::FinalBlock) -> Self {
        let genesis_block_number = genesis_block.header().number;
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
    async fn head_block(&self, _ctx: &ctx::Ctx) -> ctx::Result<validator::FinalBlock> {
        Ok(self.blocks.lock().await.head_block().clone())
    }

    async fn first_block(&self, _ctx: &ctx::Ctx) -> ctx::Result<validator::FinalBlock> {
        Ok(self.blocks.lock().await.first_block().clone())
    }

    async fn last_contiguous_block_number(
        &self,
        _ctx: &ctx::Ctx,
    ) -> ctx::Result<validator::BlockNumber> {
        Ok(self.blocks.lock().await.last_contiguous_block_number)
    }

    async fn block(
        &self,
        _ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<Option<validator::FinalBlock>> {
        Ok(self.blocks.lock().await.block(number).cloned())
    }

    async fn missing_block_numbers(
        &self,
        _ctx: &ctx::Ctx,
        range: ops::Range<validator::BlockNumber>,
    ) -> ctx::Result<Vec<validator::BlockNumber>> {
        Ok(self.blocks.lock().await.missing_block_numbers(range))
    }

    fn subscribe_to_block_writes(&self) -> watch::Receiver<validator::BlockNumber> {
        self.blocks_sender.subscribe()
    }
}

#[async_trait]
impl WriteBlockStore for InMemoryStorage {
    /// Just verifies that the payload is for the successor of the current head.
    async fn verify_payload(
        &self,
        ctx: &ctx::Ctx,
        block_number: validator::BlockNumber,
        _payload: &validator::Payload,
    ) -> ctx::Result<()> {
        let head_number = self.head_block(ctx).await?.header().number;
        if head_number >= block_number {
            return Err(anyhow::anyhow!(
                "received proposal for block {block_number:?}, while head is at {head_number:?}"
            )
            .into());
        }
        Ok(())
    }

    async fn put_block(&self, _ctx: &ctx::Ctx, block: &validator::FinalBlock) -> ctx::Result<()> {
        self.blocks.lock().await.put_block(block.clone());
        self.blocks_sender.send_replace(block.header().number);
        Ok(())
    }
}

#[async_trait]
impl ReplicaStateStore for InMemoryStorage {
    async fn replica_state(&self, _ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>> {
        Ok(self.replica_state.lock().await.clone())
    }

    async fn put_replica_state(
        &self,
        _ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> ctx::Result<()> {
        *self.replica_state.lock().await = Some(replica_state.clone());
        Ok(())
    }
}
