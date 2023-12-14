//! Node configuration.
use anyhow::Context as _;
use std::{fs, net, path::Path};
use zksync_consensus_crypto::{read_optional_text, Text, TextFmt};
use zksync_consensus_executor::{proto, ConsensusConfig, ExecutorConfig};
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::{read_optional, read_required, ProtoFmt};

/// Decodes a proto message from json for arbitrary ProtoFmt.
fn decode_json<T: ProtoFmt>(json: &str) -> anyhow::Result<T> {
    let mut d = serde_json::Deserializer::from_str(json);
    let p: T = zksync_protobuf::serde::deserialize(&mut d)?;
    d.end()?;
    Ok(p)
}

/// This struct holds the file path to each of the config files.
#[derive(Debug)]
pub struct ConfigPaths<'a> {
    /// Path to a JSON file with node configuration.
    pub config: &'a Path,
    /// Path to a validator key file.
    pub validator_key: Option<&'a Path>,
    /// Path to a node key file.
    pub node_key: &'a Path,
}

/// Node configuration including executor configuration, optional validator configuration,
/// and application-specific settings (e.g. metrics scraping).
#[derive(Debug)]
pub struct NodeConfig {
    /// Executor configuration.
    pub executor: ExecutorConfig,
    /// IP:port to serve metrics data for scraping.
    pub metrics_server_addr: Option<net::SocketAddr>,
    /// Consensus network config.
    pub consensus: Option<ConsensusConfig>,
}

impl ProtoFmt for NodeConfig {
    type Proto = proto::NodeConfig;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            executor: read_required(&r.executor).context("executor")?,
            metrics_server_addr: read_optional_text(&r.metrics_server_addr)
                .context("metrics_server_addr")?,
            consensus: read_optional(&r.consensus).context("consensus")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            executor: Some(self.executor.build()),
            metrics_server_addr: self.metrics_server_addr.as_ref().map(TextFmt::encode),
            consensus: self.consensus.as_ref().map(ProtoFmt::build),
        }
    }
}

/// Consensus network config. See `network::ConsensusConfig`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsensusConfig {
    /// Validator key of this node.
    /// It should match the secret key provided in the `validator_key` file.
    pub key: validator::PublicKey,
    /// Public TCP address that other validators are expected to connect to.
    /// It is announced over gossip network.
    pub public_addr: net::SocketAddr,
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

/// Config of the node executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutorConfig {
    /// IP:port to listen on, for incoming TCP connections.
    /// Use `0.0.0.0:<port>` to listen on all network interfaces (i.e. on all IPs exposed by this VM).
    pub server_addr: net::SocketAddr,
    /// Gossip network config.
    pub gossip: GossipConfig,
    /// Static specification of validators for Proof of Authority. Should be deprecated once we move
    /// to Proof of Stake.
    pub validators: validator::ValidatorSet,
}
/// Main struct that holds the config options for the node.
#[derive(Debug)]
pub struct Configs {
    /// Executor configuration of the node.
    pub executor: ExecutorConfig,
    /// IP:port to serve metrics data for scraping.
    pub metrics_server_addr: Option<net::SocketAddr>,
    /// Consensus-specific config extensions. Only set for validators.
    pub consensus: Option<(ConsensusConfig, validator::SecretKey)>,
    /// The validator secret key for this node.
    /// The node secret key. This key is used by both full nodes and validators to identify themselves
    /// in the P2P network.
    pub node_key: node::SecretKey,
}

impl Configs {
    /// Method to fetch the node config.
    #[tracing::instrument(level = "trace", ret)]
    pub fn read(args: ConfigPaths<'_>) -> anyhow::Result<Self> {
        let node_config = fs::read_to_string(args.config).with_context(|| {
            format!(
                "failed reading node config from `{}`",
                args.config.display()
            )
        })?;
        let node_config: NodeConfig = decode_json(&node_config).with_context(|| {
            format!(
                "failed decoding JSON node config at `{}`",
                args.config.display()
            )
        })?;

        let validator_key: Option<validator::SecretKey> = args
            .validator_key
            .as_ref()
            .map(|validator_key| {
                let read_key = fs::read_to_string(validator_key).with_context(|| {
                    format!(
                        "failed reading validator key from `{}`",
                        validator_key.display()
                    )
                })?;
                Text::new(&read_key).decode().with_context(|| {
                    format!(
                        "failed decoding validator key at `{}`",
                        validator_key.display()
                    )
                })
            })
            .transpose()?;
        let read_key = fs::read_to_string(args.node_key).with_context(|| {
            format!("failed reading node key from `{}`", args.node_key.display())
        })?;
        let node_key = Text::new(&read_key).decode().with_context(|| {
            format!("failed decoding node key at `{}`", args.node_key.display())
        })?;

        anyhow::ensure!(
            validator_key.is_some() == node_config.consensus.is_some(),
            "Validator key and consensus config must be specified at the same time"
        );
        let consensus = validator_key.and_then(|key| Some((node_config.consensus?, key)));

        let cfg = Configs {
            executor: node_config.executor,
            metrics_server_addr: node_config.metrics_server_addr,
            consensus,
            node_key,
        };
        Ok(cfg)
    }
}

impl ProtoFmt for ConsensusConfig {
    type Proto = proto::ConsensusConfig;

    fn read(proto: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            key: read_required_text(&proto.key).context("key")?,
            public_addr: read_required_text(&proto.public_addr).context("public_addr")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            key: Some(self.key.encode()),
            public_addr: Some(self.public_addr.encode()),
        }
    }
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
            validators,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            server_addr: Some(TextFmt::encode(&self.server_addr)),
            gossip: Some(self.gossip.build()),
            validators: self.validators.iter().map(|v| v.encode()).collect(),
        }
    }
}
