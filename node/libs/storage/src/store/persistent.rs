use std::fmt;

use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::{attester, validator};

use super::state::{BatchStoreState, BlockStoreState};

/// Storage of a continuous range of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait PersistentStore: 'static + fmt::Debug + Send + Sync {
    /// Genesis matching the block store content.
    /// Consensus code calls this method only once.
    async fn genesis(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis>;

    /// Return a batch by its number.
    async fn batch(&self, number: attester::BatchNumber) -> ctx::Result<attester::SyncBatch>;

    /// Range of batches persisted in storage.
    fn persisted(&self) -> sync::watch::Receiver<(BatchStoreState, BlockStoreState)>;

    /// Gets a block by its number.
    /// All the blocks from `state()` range are expected to be available.
    /// Blocks that have been queued but haven't been persisted yet don't have to be available.
    /// Returns error if block is missing.
    async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::FinalBlock>;

    /// Queue the block to be persisted in storage.
    /// `queue_next_block()` may return BEFORE the block is actually persisted,
    /// but if the call succeeded the block is expected to be persisted eventually.
    /// Implementations are only required to accept a block directly after the previous queued
    /// block, starting with `persisted().borrow().next()`.
    async fn queue_next_block(
        &self,
        ctx: &ctx::Ctx,
        block: validator::FinalBlock,
    ) -> ctx::Result<()>;

    /// Queue a batch to be persisted in storage.
    async fn queue_batch(&self, ctx: &ctx::Ctx, batch: attester::SyncBatch) -> ctx::Result<()>;
}
