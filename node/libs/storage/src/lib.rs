//! This module is responsible for persistent data storage, it provides schema-aware type-safe database access. Currently we use RocksDB,
//! but this crate only exposes an abstraction of a database, so we can easily switch to a different storage engine in the future.

mod in_memory;
mod replica_state;
#[cfg(feature = "rocksdb")]
mod rocksdb;
mod testonly;
#[cfg(test)]
mod tests;
mod traits;
mod types;

#[cfg(feature = "rocksdb")]
pub use crate::rocksdb::RocksdbStorage;
pub use crate::{
    in_memory::InMemoryStorage,
    replica_state::FallbackReplicaStateStore,
    traits::{BlockStore, ReplicaStateStore, WriteBlockStore},
    types::{Proposal, ReplicaState, StorageError, StorageResult},
};
