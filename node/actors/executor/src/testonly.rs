//! Testing extensions for node executor.
use crate::{ValidatorConfig,Config};
use rand::Rng;
use zksync_concurrency::net;
use zksync_consensus_bft::testonly::make_genesis;
use zksync_consensus_network::{testonly::Instance};
use zksync_consensus_roles::{validator};

/// Configuration for a full non-validator node.
#[derive(Debug,Clone)]
#[non_exhaustive]
pub struct FullNode {
    /// Executor configuration.
    pub config: Config,
    /// Genesis block.
    pub genesis_block: validator::FinalBlock,
}


/// Full validator configuration.
#[derive(Debug,Clone)]
#[non_exhaustive]
pub struct ValidatorNode {
    /// Full node configuration.
    pub node: FullNode,
    /// Consensus configuration of the validator.
    pub validator: ValidatorConfig,
}

impl ValidatorNode {
    /// Generates a validator config for a network with a single validator.
    pub fn for_single_validator(
        rng: &mut impl Rng,
        genesis_block_payload: validator::Payload,
        genesis_block_number: validator::BlockNumber,
    ) -> Self {
        let net_config = Instance::new_configs(rng, 1, 0).pop().unwrap();
        let validator = net_config.consensus.unwrap(); 
        let gossip = net_config.gossip;
        let (genesis_block, validators) = make_genesis(
            &[validator.key.clone()],
            genesis_block_payload,
            genesis_block_number,
        );
        let config = Config {
            server_addr: *net_config.server_addr,
            validators,
            node_key: gossip.key,
            gossip_dynamic_inbound_limit: gossip.dynamic_inbound_limit,
            gossip_static_inbound: gossip.static_inbound,
            gossip_static_outbound: gossip.static_outbound,
        };
        
        Self {
            node: FullNode {
                config,
                genesis_block,
            },
            validator,
        }
    }

    /// Creates a new full node and configures this validator to accept incoming connections from it.
    pub fn connect_full_node(&mut self, rng: &mut impl Rng) -> FullNode {
        let mut node = self.node.clone();
        node.config.server_addr = *net::tcp::testonly::reserve_listener();
        node.config.node_key = rng.gen();
        node.config.gossip_static_outbound = [(
            self.node.config.node_key.public(),
            self.node.config.server_addr,
        )].into();
        self.node.config.gossip_static_inbound.insert(node.config.node_key.public());
        node
    }
}
