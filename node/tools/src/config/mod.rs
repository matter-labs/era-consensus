//! Node configuration.

use self::config_paths::ConfigPaths;
use anyhow::Context as _;
use crypto::{read_optional_text, TextFmt};
use executor::{ConsensusConfig, ExecutorConfig};
use roles::{node, validator};
use schema::{proto::executor::config as proto, read_optional, read_required, ProtoFmt};
use std::net;

mod config_paths;

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
    pub fn read(args: &[String]) -> anyhow::Result<Self> {
        ConfigPaths::resolve(args).read()
    }
}
