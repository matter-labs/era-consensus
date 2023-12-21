//! Traits for storage.
use crate::types::ReplicaState;
use async_trait::async_trait;
use std::{sync::Arc, fmt, ops};
use zksync_concurrency::{ctx};
use zksync_consensus_roles::validator::{BlockNumber, FinalBlock, Payload};


#[async_trait]
pub trait ValidatorStore {
    fn blocks(self:Arc<Self>) -> Arc<dyn BlockStore>;
    fn replica(&self) -> &dyn ReplicaStore;

    /// Propose a payload for the block `block_number`.
    async fn propose_payload(&self, ctx: &ctx::Ctx, block_number: BlockNumber) -> ctx::Result<Payload>;

    /// Verify that `payload` is a correct proposal for the block `block_number`.
    async fn verify_payload(&self, ctx: &ctx::Ctx, block_number: BlockNumber, payload: &Payload) -> ctx::Result<()>;
}

/*
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
}*/
