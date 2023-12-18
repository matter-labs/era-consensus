//! Testing extensions for node executor.
use crate::{Config, ValidatorConfig};
use rand::Rng;
use zksync_concurrency::net;
use zksync_consensus_network::testonly::Instance;
use zksync_consensus_roles::validator;

/// Full validator configuration.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ValidatorNode {
    /// Full node configuration.
    pub node: Config,
    /// Consensus configuration of the validator.
    pub validator: ValidatorConfig,
}

impl ValidatorNode {
    /// Generates a validator config for a network with a single validator.
    pub fn for_single_validator(rng: &mut impl Rng) -> Self {
        let net_config = Instance::new_configs(rng, 1, 0).pop().unwrap();
        let validator = net_config.consensus.unwrap();
        let gossip = net_config.gossip;
        Self {
            node: Config {
                server_addr: *net_config.server_addr,
                validators: validator::ValidatorSet::new([validator.key.public()]).unwrap(),
                node_key: gossip.key,
                gossip_dynamic_inbound_limit: gossip.dynamic_inbound_limit,
                gossip_static_inbound: gossip.static_inbound,
                gossip_static_outbound: gossip.static_outbound,
            },
            validator,
        }
    }

    /// Creates a new full node and configures this validator to accept incoming connections from it.
    pub fn connect_full_node(&mut self, rng: &mut impl Rng) -> Config {
        let mut node = self.node.clone();
        node.server_addr = *net::tcp::testonly::reserve_listener();
        node.node_key = rng.gen();
        node.gossip_static_outbound = [(self.node.node_key.public(), self.node.server_addr)].into();
        self.node
            .gossip_static_inbound
            .insert(node.node_key.public());
        node
    }
}
