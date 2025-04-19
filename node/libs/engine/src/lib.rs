//! Abstraction for interaction with the execution layer.
mod block_store;
mod interface;
mod manager;
mod metrics;
pub mod testonly;
#[cfg(test)]
mod tests;

pub use crate::{
    block_store::{BlockStoreState, Last},
    interface::EngineInterface,
    manager::{EngineManager, EngineManagerRunner},
};
