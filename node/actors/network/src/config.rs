//! Network actor configs.
use std::collections::{HashMap, HashSet};
use zksync_concurrency::{limiter, net, time};
use zksync_consensus_roles::{node, validator};

/// How often we should retry to establish a connection to a validator.
/// TODO(gprusak): once it becomes relevant, choose a more appropriate retry strategy.
pub(crate) const CONNECT_RETRY: time::Duration = time::Duration::seconds(20);

/// Rate limiting config for RPCs.
#[derive(Debug, Clone)]
pub struct RpcConfig {
    /// Max rate of sending/receiving push_validator_addrs messages.
    pub push_validator_addrs_rate: limiter::Rate,
    /// Max rate of sending/receiving push_block_store_state messages.
    pub push_block_store_state_rate: limiter::Rate,
    /// Max rate of sending/receiving get_block RPCs.
    pub get_block_rate: limiter::Rate,
    /// Max rate of sending/receiving consensus messages.
    pub consensus_rate: limiter::Rate,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            push_validator_addrs_rate: limiter::Rate {
                burst: 1,
                refresh: time::Duration::seconds(5),
            },
            push_block_store_state_rate: limiter::Rate {
                burst: 2,
                refresh: time::Duration::ZERO, //milliseconds(500),
            },
            get_block_rate: limiter::Rate {
                burst: 10,
                refresh: time::Duration::ZERO, //milliseconds(100),
            },
            consensus_rate: limiter::Rate {
                burst: 10,
                refresh: time::Duration::ZERO,
            },
        }
    }
}

/// Gossip network configuration.
#[derive(Debug, Clone)]
pub struct GossipConfig {
    /// Private key of the node, every node should have one.
    pub key: node::SecretKey,
    /// Limit on the number of inbound connections outside
    /// of the `static_inbound` set.
    pub dynamic_inbound_limit: usize,
    /// Inbound connections that should be unconditionally accepted.
    pub static_inbound: HashSet<node::PublicKey>,
    /// Outbound connections that the node should actively try to
    /// establish and maintain.
    pub static_outbound: HashMap<node::PublicKey, net::Host>,
}

/// Network actor config.
#[derive(Debug, Clone)]
pub struct Config {
    /// TCP socket address to listen for inbound connections at.
    pub server_addr: net::tcp::ListenerAddr,
    /// Public TCP address that other nodes are expected to connect to.
    /// It is announced over gossip network.
    /// In case public_addr is a domain instead of ip, DNS resolution is
    /// performed and a loopback connection is established before announcing
    /// the IP address over the gossip network.
    pub public_addr: net::Host,
    /// Gossip network config.
    pub gossip: GossipConfig,
    /// Private key of the validator.
    /// None if the node is NOT a validator.
    pub validator_key: Option<validator::SecretKey>,
    /// Maximal size of the proto-encoded `validator::FinalBlock` in bytes.
    pub max_block_size: usize,
    /// If a peer doesn't respond to a ping message within `ping_timeout`,
    /// the connection is dropped.
    /// `None` disables sending ping messages (useful for tests).
    pub ping_timeout: Option<time::Duration>,
    /// Rate limiting config for RPCs.
    pub rpc: RpcConfig,
}
