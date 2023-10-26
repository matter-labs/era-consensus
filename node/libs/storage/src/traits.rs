//! Traits for storage.

use crate::{types::ReplicaState, StorageResult};
use async_trait::async_trait;
use concurrency::{ctx, sync::watch};
use roles::validator::{BlockNumber, FinalBlock};
use std::{fmt, ops, sync::Arc};

/// Storage of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait]
pub trait BlockStore: fmt::Debug + Send + Sync {
    /// Gets the head block.
    async fn head_block(&self, ctx: &ctx::Ctx) -> StorageResult<FinalBlock>;

    /// Returns a block with the least number stored in this database.
    async fn first_block(&self, ctx: &ctx::Ctx) -> StorageResult<FinalBlock>;

    /// Returns the number of the last block in the first contiguous range of blocks stored in this DB.
    /// If there are no missing blocks, this is equal to the number of [`Self::get_head_block()`],
    /// if there *are* missing blocks, the returned number will be lower.
    async fn last_contiguous_block_number(&self, ctx: &ctx::Ctx) -> StorageResult<BlockNumber>;

    /// Gets a block by its number.
    async fn block(&self, ctx: &ctx::Ctx, number: BlockNumber)
        -> StorageResult<Option<FinalBlock>>;

    /// Iterates over block numbers in the specified `range` that the DB *does not* have.
    // TODO(slowli): We might want to limit the length of the vec returned
    async fn missing_block_numbers(
        &self,
        ctx: &ctx::Ctx,
        range: ops::Range<BlockNumber>,
    ) -> StorageResult<Vec<BlockNumber>>;

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
    async fn head_block(&self, ctx: &ctx::Ctx) -> StorageResult<FinalBlock> {
        (**self).head_block(ctx).await
    }

    async fn first_block(&self, ctx: &ctx::Ctx) -> StorageResult<FinalBlock> {
        (**self).first_block(ctx).await
    }

    async fn last_contiguous_block_number(&self, ctx: &ctx::Ctx) -> StorageResult<BlockNumber> {
        (**self).last_contiguous_block_number(ctx).await
    }

    async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: BlockNumber,
    ) -> StorageResult<Option<FinalBlock>> {
        (**self).block(ctx, number).await
    }

    async fn missing_block_numbers(
        &self,
        ctx: &ctx::Ctx,
        range: ops::Range<BlockNumber>,
    ) -> StorageResult<Vec<BlockNumber>> {
        (**self).missing_block_numbers(ctx, range).await
    }

    fn subscribe_to_block_writes(&self) -> watch::Receiver<BlockNumber> {
        (**self).subscribe_to_block_writes()
    }
}

/// Mutable storage of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait]
pub trait WriteBlockStore: BlockStore {
    /// Puts a block into this storage.
    async fn put_block(&self, ctx: &ctx::Ctx, block: &FinalBlock) -> StorageResult<()>;
}

#[async_trait]
impl<S: WriteBlockStore + ?Sized> WriteBlockStore for Arc<S> {
    async fn put_block(&self, ctx: &ctx::Ctx, block: &FinalBlock) -> StorageResult<()> {
        (**self).put_block(ctx, block).await
    }
}

/// Storage for [`ReplicaState`].
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait]
pub trait ReplicaStateStore: BlockStore {
    /// Gets the replica state, if it is contained in the database.
    async fn replica_state(&self, ctx: &ctx::Ctx) -> StorageResult<Option<ReplicaState>>;

    /// Store the given replica state into the database.
    async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> StorageResult<()>;
}

#[async_trait]
impl<S: ReplicaStateStore + ?Sized> ReplicaStateStore for Arc<S> {
    async fn replica_state(&self, ctx: &ctx::Ctx) -> StorageResult<Option<ReplicaState>> {
        (**self).replica_state(ctx).await
    }

    async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> StorageResult<()> {
        (**self).put_replica_state(ctx, replica_state).await
    }
}

/// Full store combining storage of blocks and [`ReplicaState`]s.
pub trait Store: 'static + WriteBlockStore + ReplicaStateStore {
    /// Converts this store to a block store trait object.
    fn as_block_store(self: &Arc<Self>) -> Arc<dyn WriteBlockStore>
    where
        Self: Sized,
    {
        self.clone()
    }

    /// Converts this store to a replica state store trait object.
    fn as_replica_state_store(self: &Arc<Self>) -> Arc<dyn ReplicaStateStore>
    where
        Self: Sized,
    {
        self.clone()
    }
}

impl<S: 'static + WriteBlockStore + ReplicaStateStore + ?Sized> Store for S {}
