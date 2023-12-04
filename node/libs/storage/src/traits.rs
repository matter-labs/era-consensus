//! Traits for storage.
use crate::types::ReplicaState;
use async_trait::async_trait;
use std::{fmt, ops, sync::Arc};
use zksync_concurrency::{ctx, sync::watch};
use zksync_consensus_roles::validator::{BlockNumber, FinalBlock, Payload};

/// Storage of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait]
pub trait BlockStore: fmt::Debug + Send + Sync {
    /// Gets the head block.
    async fn head_block(&self, ctx: &ctx::Ctx) -> ctx::Result<FinalBlock>;

    /// Returns a block with the least number stored in this database.
    async fn first_block(&self, ctx: &ctx::Ctx) -> ctx::Result<FinalBlock>;

    /// Returns the number of the last block in the first contiguous range of blocks stored in this DB.
    /// If there are no missing blocks, this is equal to the number of [`Self::get_head_block()`],
    /// if there *are* missing blocks, the returned number will be lower.
    ///
    /// The returned number cannot underflow the [first block](Self::first_block()) stored in the DB;
    /// all blocks preceding the first block are ignored when computing this number. For example,
    /// if the storage contains blocks #5, 6 and 9, this method will return 6.
    async fn last_contiguous_block_number(&self, ctx: &ctx::Ctx) -> ctx::Result<BlockNumber>;

    /// Gets a block by its number.
    async fn block(&self, ctx: &ctx::Ctx, number: BlockNumber) -> ctx::Result<Option<FinalBlock>>;

    /// Iterates over block numbers in the specified `range` that the DB *does not* have.
    // TODO(slowli): We might want to limit the length of the vec returned
    async fn missing_block_numbers(
        &self,
        ctx: &ctx::Ctx,
        range: ops::Range<BlockNumber>,
    ) -> ctx::Result<Vec<BlockNumber>>;

    /// Subscribes to block write operations performed using this `Storage`. Note that since
    /// updates are passed using a `watch` channel, only the latest written [`BlockNumber`]
    /// will be available; intermediate updates may be dropped.
    ///
    /// If no blocks were written during the `Storage` lifetime, the channel contains the number
    /// of the genesis block.
    fn subscribe_to_block_writes(&self) -> watch::Receiver<BlockNumber>;
}

#[async_trait]
impl<S: BlockStore + ?Sized> BlockStore for Arc<S> {
    async fn head_block(&self, ctx: &ctx::Ctx) -> ctx::Result<FinalBlock> {
        (**self).head_block(ctx).await
    }

    async fn first_block(&self, ctx: &ctx::Ctx) -> ctx::Result<FinalBlock> {
        (**self).first_block(ctx).await
    }

    async fn last_contiguous_block_number(&self, ctx: &ctx::Ctx) -> ctx::Result<BlockNumber> {
        (**self).last_contiguous_block_number(ctx).await
    }

    async fn block(&self, ctx: &ctx::Ctx, number: BlockNumber) -> ctx::Result<Option<FinalBlock>> {
        (**self).block(ctx, number).await
    }

    async fn missing_block_numbers(
        &self,
        ctx: &ctx::Ctx,
        range: ops::Range<BlockNumber>,
    ) -> ctx::Result<Vec<BlockNumber>> {
        (**self).missing_block_numbers(ctx, range).await
    }

    fn subscribe_to_block_writes(&self) -> watch::Receiver<BlockNumber> {
        (**self).subscribe_to_block_writes()
    }
}

/// Mutable storage of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`ctx::Error::Canceled`].
#[async_trait]
pub trait WriteBlockStore: BlockStore {
    /// Verify that `payload` is a correct proposal for the block `block_number`.
    async fn verify_payload(
        &self,
        ctx: &ctx::Ctx,
        block_number: BlockNumber,
        _payload: &Payload,
    ) -> ctx::Result<()> {
        let head_number = self.head_block(ctx).await?.header.number;
        if head_number >= block_number {
            return Err(anyhow::anyhow!(
                "received proposal for block {block_number:?}, while head is at {head_number:?}"
            )
            .into());
        }
        Ok(())
    }
    /// Puts a block into this storage.
    async fn put_block(&self, ctx: &ctx::Ctx, block: &FinalBlock) -> ctx::Result<()>;
}

#[async_trait]
impl<S: WriteBlockStore + ?Sized> WriteBlockStore for Arc<S> {
    async fn put_block(&self, ctx: &ctx::Ctx, block: &FinalBlock) -> ctx::Result<()> {
        (**self).put_block(ctx, block).await
    }
}

/// Storage for [`ReplicaState`].
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait]
pub trait ReplicaStateStore: fmt::Debug + Send + Sync {
    /// Gets the replica state, if it is contained in the database.
    async fn replica_state(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>>;

    /// Stores the given replica state into the database.
    async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> ctx::Result<()>;
}

#[async_trait]
impl<S: ReplicaStateStore + ?Sized> ReplicaStateStore for Arc<S> {
    async fn replica_state(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>> {
        (**self).replica_state(ctx).await
    }

    async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> ctx::Result<()> {
        (**self).put_replica_state(ctx, replica_state).await
    }
}
