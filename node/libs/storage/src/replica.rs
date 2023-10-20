//! This is module is used to store the replica state. The main purpose of this is to act as a backup in case the node crashes.

use crate::{
    types::{DatabaseKey, ReplicaState},
    BlockStore, RocksdbStorage, StorageError, StorageResult,
};
use anyhow::Context as _;
use async_trait::async_trait;
use concurrency::{ctx, scope};

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
impl ReplicaStateStore for RocksdbStorage {
    async fn replica_state(&self, ctx: &ctx::Ctx) -> StorageResult<Option<ReplicaState>> {
        scope::run!(ctx, |ctx, s| async {
            Ok(s.spawn_blocking(|| {
                self.replica_state_blocking()
                    .map_err(StorageError::Database)
            })
            .join(ctx)
            .await?)
        })
        .await
    }

    async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> StorageResult<()> {
        scope::run!(ctx, |ctx, s| async {
            Ok(s.spawn_blocking(|| {
                self.put_replica_state_blocking(replica_state)
                    .map_err(StorageError::Database)
            })
            .join(ctx)
            .await?)
        })
        .await
    }
}

impl RocksdbStorage {
    fn replica_state_blocking(&self) -> anyhow::Result<Option<ReplicaState>> {
        let Some(raw_state) = self
            .read()
            .get(DatabaseKey::ReplicaState.encode_key())
            .context("Failed to get ReplicaState from RocksDB")?
        else {
            return Ok(None);
        };
        schema::decode(&raw_state)
            .map(Some)
            .context("Failed to decode replica state!")
    }

    fn put_replica_state_blocking(&self, replica_state: &ReplicaState) -> anyhow::Result<()> {
        self.write()
            .put(
                DatabaseKey::ReplicaState.encode_key(),
                schema::encode(replica_state),
            )
            .context("Failed putting ReplicaState to RocksDB")
    }
}
