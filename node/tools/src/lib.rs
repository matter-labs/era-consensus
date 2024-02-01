//! CLI tools for the consensus node.
#![allow(missing_docs)]
mod config;
pub mod k8s;
mod proto;
mod rpc;
mod store;

#[cfg(test)]
mod tests;

pub use config::{decode_json, AppConfig, ConfigPaths, NodeAddr};
pub use rpc::server;
