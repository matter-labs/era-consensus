//! CLI tools for the consensus node.
#![allow(missing_docs)]
mod config;
mod proto;
mod rpc;
mod store;

#[cfg(test)]
mod tests;

pub use rpc::server;
pub use config::{decode_json, AppConfig, ConfigPaths, NodeAddr};
