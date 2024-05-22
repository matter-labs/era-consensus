//! Abstraction for persistent data storage.
//! It provides schema-aware type-safe database access.
mod batch_store;
mod block_store;
pub mod proto;
mod replica_store;
pub mod testonly;
#[cfg(test)]
mod tests;

pub use crate::{
    batch_store::{BatchStore, BatchStoreState, PersistentBatchStore},
    block_store::{BlockStore, BlockStoreRunner, BlockStoreState, PersistentBlockStore},
    replica_store::{Proposal, ReplicaState, ReplicaStore},
};
