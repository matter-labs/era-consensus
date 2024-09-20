//! CLI tools for the consensus node.
#![allow(missing_docs)]
pub mod config;
pub mod k8s;
mod proto;
pub mod rpc;
mod store;

#[cfg(test)]
mod tests;

pub use rpc::server::RPCServer;
