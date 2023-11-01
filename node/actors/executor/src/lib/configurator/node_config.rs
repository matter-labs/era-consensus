//! Module that contains the definitions for the config files.
use anyhow::Context as _;
use crypto::{read_optional_text, read_required_text, Text, TextFmt};
use roles::{node, validator};
use schema::proto::executor::config as proto;
use protobuf::{read_required, required, ProtoFmt};
use std::{
    collections::{HashMap, HashSet},
    net,
};

/// Consensus network config. See `network::ConsensusConfig`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsensusConfig {
    /// Validator key of this node.
    /// It should match the secret key provided in the `validator_key` file.
    pub key: validator::PublicKey,
    /// Public TCP address that other validators are expected to connect to.
    /// It is announced over gossip network.
    pub public_addr: std::net::SocketAddr,
    /// Static specification of validators for Proof of Authority. Should be deprecated once we move
    /// to Proof of Stake.
    pub validators: validator::ValidatorSet,
}

impl ProtoFmt for ConsensusConfig {
    type Proto = proto::ConsensusConfig;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut validators = vec![];
        for (i, v) in r.validators.iter().enumerate() {
            validators.push(
                Text::new(v)
                    .decode()
                    .with_context(|| format!("validators[{i}]"))?,
            );
        }
        Ok(Self {
            key: read_required_text(&r.key).context("key")?,
            public_addr: read_required_text(&r.public_addr).context("public_addr")?,
            validators: validator::ValidatorSet::new(validators).context("validators")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            key: Some(self.key.encode()),
            public_addr: Some(self.public_addr.encode()),
            validators: self.validators.iter().map(|v| v.encode()).collect(),
        }
    }
}

/// Gossip network config. See `network::GossipConfig`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GossipConfig {
    /// Key of this node. It uniquely identifies the node.
    /// It should match the secret key provided in the `node_key` file.
    pub key: node::PublicKey,
    /// Limit on the number of inbound connections outside
    /// of the `static_inbound` set.
    pub dynamic_inbound_limit: u64,
    /// Inbound connections that should be unconditionally accepted.
    pub static_inbound: HashSet<node::PublicKey>,
    /// Outbound connections that the node should actively try to
    /// establish and maintain.
    pub static_outbound: HashMap<node::PublicKey, net::SocketAddr>,
}

impl ProtoFmt for GossipConfig {
    type Proto = proto::GossipConfig;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut static_inbound = HashSet::new();
        for (i, v) in r.static_inbound.iter().enumerate() {
            static_inbound.insert(
                Text::new(v)
                    .decode()
                    .with_context(|| format!("static_inbound[{i}]"))?,
            );
        }
        let mut static_outbound = HashMap::new();
        for (i, e) in r.static_outbound.iter().enumerate() {
            let key =
                read_required_text(&e.key).with_context(|| format!("static_outbound[{i}].key"))?;
            let addr = read_required_text(&e.addr)
                .with_context(|| format!("static_outbound[{i}].addr"))?;
            static_outbound.insert(key, addr);
        }
        Ok(Self {
            key: read_required_text(&r.key).context("key")?,
            dynamic_inbound_limit: *required(&r.dynamic_inbound_limit)
                .context("dynamic_inbound_limit")?,
            static_inbound,
            static_outbound,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            key: Some(self.key.encode()),
            dynamic_inbound_limit: Some(self.dynamic_inbound_limit),
            static_inbound: self.static_inbound.iter().map(TextFmt::encode).collect(),
            static_outbound: self
                .static_outbound
                .iter()
                .map(|(key, addr)| proto::NodeAddr {
                    key: Some(TextFmt::encode(key)),
                    addr: Some(TextFmt::encode(addr)),
                })
                .collect(),
        }
    }
}

/// Config of the node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeConfig {
    /// IP:port to listen on, for incoming TCP connections.
    /// Use `0.0.0.0:<port>` to listen on all network interfaces (i.e. on all IPs exposed by this VM).
    pub server_addr: net::SocketAddr,
    /// IP:port to serve metrics data for scraping.
    /// Use `0.0.0.0:<port>` to listen on all network interfaces.
    pub metrics_server_addr: Option<net::SocketAddr>,

    /// Consensus network config.
    // NOTE: we currently only support validator nodes, but eventually it will be optional.
    pub consensus: ConsensusConfig,
    /// Gossip network config.
    pub gossip: GossipConfig,

    /// Specifies the genesis block of the blockchain.
    pub genesis_block: validator::FinalBlock,
}

impl ProtoFmt for NodeConfig {
    type Proto = proto::NodeConfig;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            server_addr: read_required_text(&r.server_addr).context("server_addr")?,
            metrics_server_addr: read_optional_text(&r.metrics_server_addr)
                .context("metrics_server_addr")?,
            consensus: read_required(&r.consensus).context("consensus")?,
            gossip: read_required(&r.gossip).context("gossip")?,
            genesis_block: read_required_text(&r.genesis_block).context("genesis_block")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            server_addr: Some(TextFmt::encode(&self.server_addr)),
            metrics_server_addr: self.metrics_server_addr.as_ref().map(TextFmt::encode),
            consensus: Some(self.consensus.build()),
            gossip: Some(self.gossip.build()),
            genesis_block: Some(TextFmt::encode(&self.genesis_block)),
        }
    }
}