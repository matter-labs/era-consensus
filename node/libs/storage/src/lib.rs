//! Abstraction for persistent data storage.
//! It provides schema-aware type-safe database access.
pub mod proto;
mod replica_store;
mod store;
pub mod testonly;
#[cfg(test)]
mod tests;

pub use crate::{
    replica_store::{Proposal, ReplicaState, ReplicaStore},
    store::BatchStoreState,
    store::BlockStore,
    store::BlockStoreState,
    store::PersistentStore,
    store::StoreRunner,
};
