//! CLI tools for the consensus node.
#![allow(missing_docs)]
mod config;
pub mod k8s;
mod proto;
pub mod rpc;
mod store;

#[cfg(test)]
mod tests;

pub use config::{decode_json, AppConfig, ConfigPaths, NodeAddr, NODES_PORT};
pub use rpc::server::RPCServer;
