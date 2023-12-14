//! Module to create the configuration for the consensus node.
use anyhow::Context as _;
use std::{
    collections::{HashMap, HashSet},
    net,
};
use zksync_consensus_crypto::{read_required_text, Text, TextFmt};
use zksync_consensus_network::gossip;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::{read_required, required, ProtoFmt};

pub mod proto;
#[cfg(test)]
mod tests;

/// Config of the node executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutorConfig {
    /// IP:port to listen on, for incoming TCP connections.
    /// Use `0.0.0.0:<port>` to listen on all network interfaces (i.e. on all IPs exposed by this VM).
    pub server_addr: net::SocketAddr,
    /// Static specification of validators for Proof of Authority. Should be deprecated once we move
    /// to Proof of Stake.
    pub validators: validator::ValidatorSet,

    /// Key of this node. It uniquely identifies the node.
    /// It should match the secret key provided in the `node_key` file.
    pub node_key: node::SecretKey,
    /// Limit on the number of inbound connections outside
    /// of the `static_inbound` set.
    pub gossip_dynamic_inbound_limit: u64,
    /// Inbound connections that should be unconditionally accepted.
    pub gossip_static_inbound: HashSet<node::PublicKey>,
    /// Outbound connections that the node should actively try to
    /// establish and maintain.
    pub gossip_static_outbound: HashMap<node::PublicKey, net::SocketAddr>,
}
