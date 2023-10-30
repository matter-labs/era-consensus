//! This module is responsible for persistent data storage, it provides schema-aware type-safe database access. Currently we use RocksDB,
//! but this crate only exposes an abstraction of a database, so we can easily switch to a different storage engine in the future.

mod replica_state;
mod rocksdb;
mod testonly;
#[cfg(test)]
mod tests;
mod traits;
mod types;

pub use crate::{
    replica_state::FallbackReplicaStateStore,
    rocksdb::RocksdbStorage,
    traits::{BlockStore, ReplicaStateStore, WriteBlockStore},
    types::{Proposal, ReplicaState, StorageError, StorageResult},
};
