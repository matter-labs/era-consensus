//! CLI tools for the consensus node.
#![allow(missing_docs)]
mod config;
mod proto;
mod store;

#[cfg(test)]
mod tests;

pub use config::{AppConfig, ConfigPaths};
