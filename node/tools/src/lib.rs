//! CLI tools for the consensus node.
#![allow(missing_docs)]
pub mod config;
mod engine;
mod proto;
pub mod rpc;

#[cfg(test)]
mod tests;

pub use rpc::server::RPCServer;
