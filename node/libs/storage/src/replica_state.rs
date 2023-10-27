//! `FallbackReplicaStateStore` type.

use crate::{
    traits::{BlockStore, ReplicaStateStore},
    types::{ReplicaState, StorageResult},
};
use concurrency::ctx;
use roles::validator;
use std::sync::Arc;

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
pub struct FallbackReplicaStateStore {
    base: Arc<dyn ReplicaStateStore>,
    fallback: Arc<dyn BlockStore>,
}

impl FallbackReplicaStateStore {
    /// Creates a store from a type implementing both replica state and block storage.
    pub fn from_store<S>(store: Arc<S>) -> Self
    where
        S: ReplicaStateStore + BlockStore + 'static,
    {
        Self {
            base: store.clone(),
            fallback: store,
        }
    }

    /// Creates a new replica state store with a fallback.
    pub fn new(base: Arc<dyn ReplicaStateStore>, fallback: Arc<dyn BlockStore>) -> Self {
        Self { base, fallback }
    }

    /// Gets the replica state. If it's not present, falls back to recover it from the fallback block store.
    pub async fn replica_state(&self, ctx: &ctx::Ctx) -> StorageResult<ReplicaState> {
        let replica_state = self.base.replica_state(ctx).await?;
        if let Some(replica_state) = replica_state {
            Ok(replica_state)
        } else {
            let head_block = self.fallback.head_block(ctx).await?;
            Ok(ReplicaState::from(head_block.justification))
        }
    }

    /// Stores the given replica state into the database. This just proxies to the base replica store.
    pub async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> StorageResult<()> {
        self.base.put_replica_state(ctx, replica_state).await
    }
}
