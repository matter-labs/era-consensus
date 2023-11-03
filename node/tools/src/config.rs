//! Node configuration.

use anyhow::Context as _;
use crypto::{read_optional_text, Text, TextFmt};
use executor::{ConsensusConfig, ExecutorConfig};
use roles::{node, validator};
use schema::proto::executor::config as proto;
use std::{fs, net, path::Path};
use zksync_protobuf::{read_optional, read_required, ProtoFmt};

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
        let node_config: NodeConfig =
            zksync_protobuf::decode_json(&node_config).with_context(|| {
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
