use std::fmt;

use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::validator;

use crate::BlockStoreState;

/// Defines the interface between the consensus layer and the execution layer.
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait EngineInterface: 'static + fmt::Debug + Send + Sync {
    /// Genesis matching the current chain.
    /// Consensus code calls this method only once.
    async fn genesis(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis>;

    /// Range of blocks persisted in storage.
    fn persisted(&self) -> sync::watch::Receiver<BlockStoreState>;

    /// Gets the validator committee that is active at the given block number.
    async fn get_validator_committee(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Committee>;

    /// Gets the pending validator committee (if one exists) and the block number
    /// at which it will become active.
    async fn get_pending_validator_committee(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<(validator::Committee, validator::BlockNumber)>>;

    /// Gets a block by its number.
    /// All the blocks from `persisted()` range are expected to be available.
    /// Blocks that have been queued but haven't been persisted yet don't have to be available.
    /// Returns error if block is missing.
    async fn get_block(
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

    /// Verifies a pre-genesis block.
    /// It may interpret `block.justification`
    /// and/or consult external source of truth.
    async fn verify_pregenesis_block(
        &self,
        ctx: &ctx::Ctx,
        block: &validator::PreGenesisBlock,
    ) -> ctx::Result<()>;

    /// Used by replica to verify a payload for the next block proposed by the leader.
    async fn verify_payload(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
        payload: &validator::Payload,
    ) -> ctx::Result<()>;

    /// Used by leader to propose a payload for the next block.
    async fn propose_payload(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload>;

    /// Gets the replica state, if it is contained in the database. Otherwise, returns the default
    /// state.
    async fn get_state(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::ReplicaState>;

    /// Stores the given replica state into the database.
    async fn set_state(&self, ctx: &ctx::Ctx, state: &validator::ReplicaState) -> ctx::Result<()>;
}
