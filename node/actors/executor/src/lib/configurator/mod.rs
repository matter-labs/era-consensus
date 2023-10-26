//! Module to create the configuration for the consensus node.

use roles::{node, validator};
use tracing::instrument;

mod config_paths;
pub(super) mod node_config; // FIXME: flatten

#[cfg(test)]
mod tests;

/// Main struct that holds the config options for the node.
// FIXME: remove from library
#[derive(Debug)]
pub struct Configs {
    /// This config file describes the environment for the node.
    pub config: node_config::NodeConfig,
    /// The validator secret key for this node.
    pub validator_key: Option<validator::SecretKey>,
    /// The node secret key. This key is used by both full nodes and validators to identify themselves
    /// in the P2P network.
    pub node_key: node::SecretKey,
}

impl Configs {
    /// Method to fetch the node config.
    #[instrument(level = "trace", ret)]
    pub fn read(args: &[String]) -> anyhow::Result<Self> {
        config_paths::ConfigPaths::resolve(args).read()
    }
}
