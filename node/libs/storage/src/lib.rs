//! This module is responsible for persistent data storage, it provides schema-aware type-safe database access. Currently we use RocksDB,
//! but this crate only exposes an abstraction of a database, so we can easily switch to a different storage engine in the future.

mod block_store;
pub mod proto;
mod replica_store;
pub mod testonly;
#[cfg(test)]
mod tests;

pub use crate::{
    block_store::{BlockStore, BlockStoreRunner, BlockStoreState, PersistentBlockStore},
    replica_store::{Proposal, ReplicaState, ReplicaStore},
};
