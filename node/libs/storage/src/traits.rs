//! Traits for storage.
use crate::types::ReplicaState;
use async_trait::async_trait;
use std::{fmt, ops};
use zksync_concurrency::{ctx};
use zksync_consensus_roles::validator::{BlockNumber, FinalBlock, Payload};

/// Storage of a continuous range of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait]
pub trait PersistentBlockStore: fmt::Debug + Send + Sync {
    /// Range of blocks avaliable in storage.
    /// Consensus code calls this method only once and then tracks the
    /// range of avaliable blocks internally.
    async fn available_blocks(&self, ctx: &ctx::Ctx) -> ctx::Result<ops::Range<BlockNumber>>;

    /// Gets a block by its number. Should returns an error if block is not available.
    async fn block(&self, ctx: &ctx::Ctx, number: BlockNumber) -> ctx::Result<FinalBlock>;

    /// Persistently store a block.
    /// Implementations are only required to accept a block directly after the current last block,
    /// so that the stored blocks always constitute a continuous range.
    /// Implementation should return only after the block is stored PERSISTENTLY -
    /// consensus liveness property depends on this behavior.
    async fn store_next_block(&self, ctx: &ctx::Ctx, block: &FinalBlock) -> ctx::Result<()>;
}

/// Storage for [`ReplicaState`].
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait]
pub trait ValidatorStore: fmt::Debug + Send + Sync {
    /// Gets the replica state, if it is contained in the database.
    async fn replica_state(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>>;

    /// Stores the given replica state into the database.
    async fn set_replica_state(&self, ctx: &ctx::Ctx, replica_state: &ReplicaState) -> ctx::Result<()>;

    /// Propose a payload for the block `block_number`.
    async fn propose_payload(&self, ctx: &ctx::Ctx, block_number: BlockNumber) -> ctx::Result<Payload>;

    /// Verify that `payload` is a correct proposal for the block `block_number`.
    async fn verify_payload(&self, ctx: &ctx::Ctx, block_number: BlockNumber, payload: &Payload) -> ctx::Result<()>;
}

#[async_trait]
pub trait ValidatorStoreDefault : Send + Sync {
    fn inner(&self) -> &dyn ValidatorStore;
    async fn replica_state(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>> { self.inner().replica_state(ctx).await }
    async fn set_replica_state(&self, ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()> { self.inner().set_replica_state(ctx,state).await }
    async fn propose_payload(&self, ctx: &ctx::Ctx, block_number: BlockNumber) -> ctx::Result<Payload> { self.inner().propose_payload(ctx,block_number).await }
    async fn verify_payload(&self, ctx: &ctx::Ctx, block_number: BlockNumber, payload: &Payload) -> ctx::Result<()> { self.inner().verify_payload(ctx,block_number,payload).await }
}

#[async_trait]
impl<T:ValidatorStoreDefault + fmt::Debug> ValidatorStore for T {
    async fn replica_state(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>> { ValidatorStoreDefault::replica_state(self,ctx).await }
    async fn set_replica_state(&self, ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()> { ValidatorStoreDefault::set_replica_state(self,ctx,state).await }
    async fn propose_payload(&self, ctx: &ctx::Ctx, block_number: BlockNumber) -> ctx::Result<Payload> { ValidatorStoreDefault::propose_payload(self,ctx,block_number).await }
    async fn verify_payload(&self, ctx: &ctx::Ctx, block_number: BlockNumber, payload: &Payload) -> ctx::Result<()> { ValidatorStoreDefault::verify_payload(self,ctx,block_number,payload).await }
}
