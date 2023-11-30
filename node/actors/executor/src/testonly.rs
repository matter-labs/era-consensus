//! Testing extensions for node executor.
use crate::config::{ConsensusConfig, ExecutorConfig, GossipConfig};
use rand::Rng;
use std::collections::HashMap;
use zksync_concurrency::net;
use zksync_consensus_bft::testonly::make_genesis;
use zksync_consensus_network::{consensus, testonly::Instance};
use zksync_consensus_roles::{
    node,
    validator::{self, Payload},
};

impl ConsensusConfig {
    fn from_network_config(
        src: consensus::Config,
        protocol_version: validator::ProtocolVersion,
    ) -> Self {
        Self {
            key: src.key.public(),
            public_addr: src.public_addr,
            protocol_version,
        }
    }
}

/// Full validator configuration.
#[derive(Debug)]
#[non_exhaustive]
pub struct FullValidatorConfig {
    /// Executor configuration.
    pub node_config: ExecutorConfig,
    /// Secret key of the node used for identification in the gossip network.
    pub node_key: node::SecretKey,
    /// Consensus configuration of the validator.
    pub consensus_config: ConsensusConfig,
    /// Secret key for consensus.
    pub validator_key: validator::SecretKey,
}

impl FullValidatorConfig {
    /// Generates a validator config for a network with a single validator.
    ///
    /// `protocol_version` is used both for the genesis block and as the current protocol version.
    pub fn for_single_validator(
        rng: &mut impl Rng,
        protocol_version: validator::ProtocolVersion,
        genesis_block_payload: Payload,
    ) -> Self {
        let mut net_configs = Instance::new_configs(rng, 1, 0);
        assert_eq!(net_configs.len(), 1);
        let net_config = net_configs.pop().unwrap();
        let consensus_config = net_config.consensus.unwrap();
        let validator_key = consensus_config.key.clone();
        let consensus_config =
            ConsensusConfig::from_network_config(consensus_config, protocol_version);

        let (genesis_block, validators) = make_genesis(
            &[validator_key.clone()],
            protocol_version,
            genesis_block_payload,
        );
        let node_key = net_config.gossip.key.clone();
        let node_config = ExecutorConfig {
            server_addr: *net_config.server_addr,
            gossip: net_config.gossip.into(),
            genesis_block,
            validators,
        };

        Self {
            node_config,
            node_key,
            consensus_config,
            validator_key,
        }
    }

    /// Creates a new full node and configures this validator to accept incoming connections from it.
    pub fn connect_full_node(&mut self, rng: &mut impl Rng) -> FullNodeConfig {
        let full_node_config = FullNodeConfig::new(rng, self);
        self.node_config
            .gossip
            .static_inbound
            .insert(full_node_config.node_key.public());
        full_node_config
    }
}

/// Configuration for a full non-validator node.
#[derive(Debug)]
#[non_exhaustive]
pub struct FullNodeConfig {
    /// Executor configuration.
    pub node_config: ExecutorConfig,
    /// Secret key of the node used for identification in the gossip network.
    pub node_key: node::SecretKey,
}

impl FullNodeConfig {
    fn new(rng: &mut impl Rng, validator: &FullValidatorConfig) -> Self {
        let node_key: node::SecretKey = rng.gen();
        let full_node_addr = net::tcp::testonly::reserve_listener();
        let node_config = ExecutorConfig {
            server_addr: *full_node_addr,
            gossip: GossipConfig {
                key: node_key.public(),
                static_outbound: HashMap::from([(
                    validator.node_key.public(),
                    validator.node_config.server_addr,
                )]),
                ..validator.node_config.gossip.clone()
            },
            ..validator.node_config.clone()
        };

        Self {
            node_config,
            node_key,
        }
    }
}
