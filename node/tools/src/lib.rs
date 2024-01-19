//! CLI tools for the consensus node.
#![allow(missing_docs)]
mod config;
mod proto;
mod rpc;
mod store;

#[cfg(test)]
mod tests;

pub use config::{AppConfig, ConfigPaths};
pub use rpc::server;
