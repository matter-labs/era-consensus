use std::fmt;

use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::{node, validator};

use crate::BlockStoreState;

/// Defines the interface between the consensus layer and the execution layer.
/// Implementations **must** propagate context cancellation.
#[async_trait::async_trait]
pub trait EngineInterface: 'static + fmt::Debug + Send + Sync {
    /// Genesis matching the current chain.
    /// Consensus code calls this method only once.
    async fn genesis(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis>;

    /// Gets the validator schedule that is active at the given block number together with
    /// the block number at which it became active.
    async fn get_validator_schedule(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<(validator::Schedule, validator::BlockNumber)>;

    /// Gets the pending validator schedule (if one exists) at the given block number together with
    /// the block number at which it will become active.
    async fn get_pending_validator_schedule(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<Option<(validator::Schedule, validator::BlockNumber)>>;

    /// Range of blocks persisted in storage.
    fn persisted(&self) -> sync::watch::Receiver<BlockStoreState>;

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

    /// Pushes a transaction to the mempool. Returns `true` if the transaction was accepted, `false` otherwise.
    async fn push_tx(&self, ctx: &ctx::Ctx, tx: node::Transaction) -> ctx::Result<bool>;

    /// Fetches new transactions from the mempool. Returns a list of new transactions that need to be propagated.
    async fn fetch_txs(&self, ctx: &ctx::Ctx) -> ctx::Result<Vec<node::Transaction>>;
}
