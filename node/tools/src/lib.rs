//! CLI tools for the consensus node.
#![allow(missing_docs)]
mod config;
mod proto;

#[cfg(test)]
mod tests;

pub use config::{ConfigPaths,AppConfig};
