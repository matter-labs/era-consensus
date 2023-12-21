//! This module is responsible for persistent data storage, it provides schema-aware type-safe database access. Currently we use RocksDB,
//! but this crate only exposes an abstraction of a database, so we can easily switch to a different storage engine in the future.
#![allow(missing_docs)]

pub mod proto;
#[cfg(feature = "rocksdb")]
pub mod rocksdb;
pub mod testonly;
#[cfg(test)]
mod tests;
mod block_store;
mod replica_store;

pub use crate::block_store::{PersistentBlockStore,BlockStore};
pub use crate::replica_store::{ReplicaStore,ReplicaState,Proposal};
