//! Abstraction for persistent data storage.
//! It provides schema-aware type-safe database access.
mod block_store;
mod interface;
pub mod proto;
pub mod testonly;
#[cfg(test)]
mod tests;

pub use crate::{
    block_store::{BlockStore, BlockStoreRunner, BlockStoreState, Last, PersistentBlockStore},
    interface::{ChonkyV2State, Proposal, ReplicaState, ReplicaStore},
};
