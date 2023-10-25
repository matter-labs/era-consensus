//! Module to create the configuration for the consensus node.
use concurrency::net;
use roles::{node, validator};
use tracing::instrument;

mod config_paths;
pub mod node_config;

#[cfg(test)]
mod tests;

/// Main struct that holds the config options for the node.
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

    fn gossip_config(&self) -> network::gossip::Config {
        let gossip = &self.config.gossip;
        network::gossip::Config {
            key: self.node_key.clone(),
            dynamic_inbound_limit: gossip.dynamic_inbound_limit,
            static_inbound: gossip.static_inbound.clone(),
            static_outbound: gossip.static_outbound.clone(),
            enable_pings: true,
        }
    }

    fn consensus_config(&self) -> Option<network::consensus::Config> {
        let consensus = self.config.consensus.as_ref()?;
        Some(network::consensus::Config {
            // Consistency of the validator key has been verified in constructor.
            key: self.validator_key.clone().unwrap(),
            public_addr: consensus.public_addr,
            validators: self.config.validators.clone(),
        })
    }

    /// Extracts a network crate config.
    pub fn network_config(&self) -> network::Config {
        network::Config {
            server_addr: net::tcp::ListenerAddr::new(self.config.server_addr),
            gossip: self.gossip_config(),
            consensus: self.consensus_config(),
        }
    }
}
