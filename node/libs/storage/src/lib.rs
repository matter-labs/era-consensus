//! Abstraction for persistent data storage.
//! It provides schema-aware type-safe database access.
mod block_store;
pub mod proto;
mod replica_store;
pub mod testonly;
#[cfg(test)]
mod tests;

pub use crate::{
    block_store::{Last, BlockStore, BlockStoreRunner, BlockStoreState, PersistentBlockStore},
    replica_store::{Proposal, ReplicaState, ReplicaStore},
};
