//! Defines storage layer for persistent replica state.
use std::fmt;

use anyhow::Context as _;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;
use zksync_protobuf::{read_optional, read_required, required, ProtoFmt};

use crate::proto;

/// Storage of a continuous range of L2 blocks.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait EngineInterface: 'static + fmt::Debug + Send + Sync {
    /// Genesis matching the block store content.
    /// Consensus code calls this method only once.
    async fn genesis(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis>;

    /// Range of blocks persisted in storage.
    fn persisted(&self) -> sync::watch::Receiver<BlockStoreState>;

    /// Verifies a pre-genesis block.
    /// It may interpret `block.justification`
    /// and/or consult external source of truth.
    async fn verify_pregenesis_block(
        &self,
        ctx: &ctx::Ctx,
        block: &validator::PreGenesisBlock,
    ) -> ctx::Result<()>;

    /// Gets a block by its number.
    /// All the blocks from `state()` range are expected to be available.
    /// Blocks that have been queued but haven't been persisted yet don't have to be available.
    /// Returns error if block is missing.
    async fn block(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Block>;

    /// Queue the block to be persisted in storage.
    /// `queue_next_block()` may return BEFORE the block is actually persisted,
    /// but if the call succeeded the block is expected to be persisted eventually.
    /// Implementations are only required to accept a block directly after the previous queued
    /// block, starting with `persisted().borrow().next()`.
    async fn queue_next_block(&self, ctx: &ctx::Ctx, block: validator::Block) -> ctx::Result<()>;

    /// Used by leader to propose a payload for the next block.
    async fn propose(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload>;
    /// Used by replica to verify a payload for the next block proposed by the leader.
    async fn verify(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
        payload: &validator::Payload,
    ) -> ctx::Result<()>;

    /// Gets the replica state, if it is contained in the database. Otherwise, returns the default
    /// state.
    async fn state(&self, ctx: &ctx::Ctx) -> ctx::Result<ReplicaState>;

    /// Stores the given replica state into the database.
    async fn set_state(&self, ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()>;
}
