//! This is module is used to store the replica state. The main purpose of this is to act as a backup in case the node crashes.

use crate::{
    types::{DatabaseKey, ReplicaState},
    BlockStore, RocksdbStorage,
};
use async_trait::async_trait;
use concurrency::{ctx, scope};

/// Storage for [`ReplicaState`].
#[async_trait]
pub trait ReplicaStateStore: BlockStore {
    /// Gets the replica state, if it is contained in the database.
    async fn replica_state(&self, ctx: &ctx::Ctx) -> anyhow::Result<Option<ReplicaState>>;

    /// Store the given replica state into the database.
    async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> anyhow::Result<()>;
}

#[async_trait]
impl ReplicaStateStore for RocksdbStorage {
    async fn replica_state(&self, ctx: &ctx::Ctx) -> anyhow::Result<Option<ReplicaState>> {
        scope::run!(ctx, |ctx, s| async {
            s.spawn_blocking(|| Ok(self.replica_state_blocking()))
                .join(ctx)
                .await
        })
        .await
        .map_err(Into::into)
    }

    async fn put_replica_state(
        &self,
        ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> anyhow::Result<()> {
        scope::run!(ctx, |ctx, s| async {
            s.spawn_blocking(|| {
                self.put_replica_state_blocking(replica_state);
                Ok(())
            })
            .join(ctx)
            .await
        })
        .await
        .map_err(Into::into)
    }
}

impl RocksdbStorage {
    fn replica_state_blocking(&self) -> Option<ReplicaState> {
        self.read()
            .get(DatabaseKey::ReplicaState.encode_key())
            .unwrap()
            .map(|b| schema::decode(&b).expect("Failed to decode replica state!"))
    }

    fn put_replica_state_blocking(&self, replica_state: &ReplicaState) {
        self.write()
            .put(
                DatabaseKey::ReplicaState.encode_key(),
                schema::encode(replica_state),
            )
            .unwrap();
    }
}
