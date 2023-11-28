//! Module to create the configuration for the consensus node.
use anyhow::Context as _;
use std::{
    collections::{HashMap, HashSet},
    net,
};
use zksync_consensus_bft::misc::consensus_threshold;
use zksync_consensus_crypto::{read_required_text, Text, TextFmt};
use zksync_consensus_network::gossip;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::{read_required, required, ProtoFmt};

pub mod proto;
#[cfg(test)]
mod tests;

/// Consensus network config. See `network::ConsensusConfig`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsensusConfig {
    /// Validator key of this node.
    /// It should match the secret key provided in the `validator_key` file.
    pub key: validator::PublicKey,
    /// Public TCP address that other validators are expected to connect to.
    /// It is announced over gossip network.
    pub public_addr: net::SocketAddr,
    /// Currently used protocol version for consensus messages.
    pub protocol_version: validator::ProtocolVersion,
}

impl ProtoFmt for ConsensusConfig {
    type Proto = proto::ConsensusConfig;

    fn read(proto: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            key: read_required_text(&proto.key).context("key")?,
            public_addr: read_required_text(&proto.public_addr).context("public_addr")?,
            protocol_version: required(&proto.protocol_version)
                .copied()
                .and_then(validator::ProtocolVersion::try_from)
                .context("protocol_version")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            key: Some(self.key.encode()),
            public_addr: Some(self.public_addr.encode()),
            protocol_version: Some(self.protocol_version.as_u32()),
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

impl From<gossip::Config> for GossipConfig {
    fn from(config: gossip::Config) -> Self {
        Self {
            key: config.key.public(),
            dynamic_inbound_limit: config.dynamic_inbound_limit,
            static_inbound: config.static_inbound,
            static_outbound: config.static_outbound,
        }
    }
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

/// Config of the node executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutorConfig {
    /// IP:port to listen on, for incoming TCP connections.
    /// Use `0.0.0.0:<port>` to listen on all network interfaces (i.e. on all IPs exposed by this VM).
    pub server_addr: net::SocketAddr,
    /// Gossip network config.
    pub gossip: GossipConfig,
    /// Specifies the genesis block of the blockchain.
    pub genesis_block: validator::FinalBlock,
    /// Static specification of validators for Proof of Authority. Should be deprecated once we move
    /// to Proof of Stake.
    pub validators: validator::ValidatorSet,
}

impl ExecutorConfig {
    /// Validates internal consistency of this config.
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        let consensus_threshold = consensus_threshold(self.validators.len());
        self.genesis_block
            .validate(&self.validators, consensus_threshold)?;
        Ok(())
    }
}

impl ProtoFmt for ExecutorConfig {
    type Proto = proto::ExecutorConfig;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let validators = r.validators.iter().enumerate().map(|(i, v)| {
            Text::new(v)
                .decode()
                .with_context(|| format!("validators[{i}]"))
        });
        let validators: anyhow::Result<Vec<_>> = validators.collect();
        let validators = validator::ValidatorSet::new(validators?).context("validators")?;

        Ok(Self {
            server_addr: read_required_text(&r.server_addr).context("server_addr")?,
            gossip: read_required(&r.gossip).context("gossip")?,
            genesis_block: read_required_text(&r.genesis_block).context("genesis_block")?,
            validators,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            server_addr: Some(TextFmt::encode(&self.server_addr)),
            gossip: Some(self.gossip.build()),
            genesis_block: Some(TextFmt::encode(&self.genesis_block)),
            validators: self.validators.iter().map(|v| v.encode()).collect(),
        }
    }
}
