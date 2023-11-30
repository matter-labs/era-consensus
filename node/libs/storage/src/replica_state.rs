//! `FallbackReplicaStateStore` type.

use crate::{
    traits::{ReplicaStateStore, WriteBlockStore},
    types::ReplicaState,
};
use std::sync::Arc;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;

impl From<validator::CommitQC> for ReplicaState {
    fn from(certificate: validator::CommitQC) -> Self {
        Self {
            view: certificate.message.view,
            phase: validator::Phase::Prepare,
            high_vote: certificate.message,
            high_qc: certificate,
            proposals: vec![],
        }
    }
}

/// [`ReplicaStateStore`] wrapper that falls back to a specified block store.
#[derive(Debug, Clone)]
pub struct ReplicaStore {
    state: Arc<dyn ReplicaStateStore>,
    blocks: Arc<dyn WriteBlockStore>,
}

impl ReplicaStore {
    /// Creates a store from a type implementing both replica state and block storage.
    pub fn from_store<S>(store: Arc<S>) -> Self
    where
        S: ReplicaStateStore + WriteBlockStore + 'static,
    {
        Self {
            state: store.clone(),
            blocks: store,
        }
    }

    /// Creates a new replica state store with a fallback.
    pub fn new(state: Arc<dyn ReplicaStateStore>, blocks: Arc<dyn WriteBlockStore>) -> Self {
        Self { state, blocks }
    }

    /// Gets the replica state. If it's not present, falls back to recover it from the fallback block store.
    pub async fn replica_state(&self, ctx: &ctx::Ctx) -> ctx::Result<ReplicaState> {
        let replica_state = self.state.replica_state(ctx).await?;
        if let Some(replica_state) = replica_state {
            Ok(replica_state)
        } else {
            let head_block = self.blocks.head_block(ctx).await?;
            Ok(ReplicaState::from(head_block.justification))
        }
    }

    /// Stores the given replica state into the database. This just proxies to the base replica store.
    pub async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> ctx::Result<()> {
        self.state.put_replica_state(ctx, replica_state).await
    }

    /// Puts a block into this storage.
    pub async fn put_block(
        &self,
        ctx: &ctx::Ctx,
        block: &validator::FinalBlock,
    ) -> ctx::Result<()> {
        self.blocks.put_block(ctx, block).await
    }
}
