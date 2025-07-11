//! Node configuration.
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
};

use anyhow::Context as _;
use zksync_concurrency::{ctx, net, time};
use zksync_consensus_crypto::{read_optional_text, read_required_text, Text, TextFmt};
use zksync_consensus_engine::{
    testonly::in_memory, EngineInterface, EngineManager, EngineManagerRunner,
};
use zksync_consensus_executor::{self as executor};
use zksync_consensus_network as network;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::{read_optional, read_required, required, ProtoFmt};

use crate::{engine, proto};

const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

fn read_required_secret_text<T: TextFmt>(text: &Option<String>) -> anyhow::Result<T> {
    Text::new(
        text.as_ref()
            .ok_or_else(|| anyhow::format_err!("missing"))?,
    )
    .decode()
    .map_err(|_| anyhow::format_err!("invalid format"))
}

fn read_optional_secret_text<T: TextFmt>(text: &Option<String>) -> anyhow::Result<Option<T>> {
    text.as_ref()
        .map(|t| Text::new(t).decode())
        .transpose()
        .map_err(|_| anyhow::format_err!("invalid format"))
}

/// Ports for the nodes to listen on kubernetes pod.
pub const NODES_PORT: u16 = 3054;

/// Pair of (public key, host addr) for a gossip network node.
#[derive(Debug, Clone)]
pub struct NodeAddr {
    pub key: node::PublicKey,
    pub addr: net::Host,
}

impl ProtoFmt for NodeAddr {
    type Proto = proto::NodeAddr;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let key = read_required_text(&r.key).context("key")?;
        let addr = net::Host(required(&r.addr).context("addr")?.clone());
        Ok(Self { addr, key })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            key: Some(TextFmt::encode(&self.key)),
            addr: Some(self.addr.0.clone()),
        }
    }
}

/// Node configuration including executor configuration, optional validator configuration,
/// and application-specific settings (e.g. metrics scraping).
#[derive(Debug, PartialEq, Clone)]
pub struct App {
    pub server_addr: SocketAddr,
    pub public_addr: net::Host,
    pub rpc_addr: Option<SocketAddr>,
    pub metrics_server_addr: Option<SocketAddr>,

    pub genesis: validator::Genesis,
    pub max_payload_size: usize,
    pub max_tx_size: usize,
    pub view_timeout: time::Duration,
    pub validator_key: Option<validator::SecretKey>,

    pub node_key: node::SecretKey,
    pub gossip_dynamic_inbound_limit: usize,
    pub gossip_static_inbound: HashSet<node::PublicKey>,
    pub gossip_static_outbound: HashMap<node::PublicKey, net::Host>,

    pub debug_page: Option<DebugPage>,

    pub fetch_schedule_interval: time::Duration,
}

impl ProtoFmt for DebugPage {
    type Proto = proto::DebugPageConfig;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            addr: read_required_text(&r.addr).context("addr")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            addr: Some(self.addr.encode()),
        }
    }
}

impl ProtoFmt for App {
    type Proto = proto::AppConfig;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut gossip_static_inbound = HashSet::new();
        for (i, v) in r.gossip_static_inbound.iter().enumerate() {
            gossip_static_inbound.insert(
                Text::new(v)
                    .decode()
                    .with_context(|| format!("gossip_static_inbound[{i}]"))?,
            );
        }

        let mut gossip_static_outbound = HashMap::new();
        for (i, e) in r.gossip_static_outbound.iter().enumerate() {
            let node_addr: NodeAddr =
                ProtoFmt::read(e).with_context(|| format!("gossip_static_outbound[{i}]"))?;
            gossip_static_outbound.insert(node_addr.key, node_addr.addr);
        }

        let max_payload_size = required(&r.max_payload_size)
            .and_then(|x| Ok((*x).try_into()?))
            .context("max_payload_size")?;

        let max_tx_size = required(&r.max_tx_size)
            .and_then(|x| Ok((*x).try_into()?))
            .context("max_tx_size")?;

        Ok(Self {
            server_addr: read_required_text(&r.server_addr).context("server_addr")?,
            public_addr: net::Host(required(&r.public_addr).context("public_addr")?.clone()),
            rpc_addr: read_optional_text(&r.rpc_addr).context("rpc_addr")?,
            metrics_server_addr: read_optional_text(&r.metrics_server_addr)
                .context("metrics_server_addr")?,

            genesis: read_required(&r.genesis).context("genesis")?,
            max_payload_size,
            max_tx_size,
            view_timeout: read_required(&r.view_timeout).context("view_timeout")?,
            // TODO: read secret.
            validator_key: read_optional_secret_text(&r.validator_secret_key)
                .context("validator_secret_key")?,

            node_key: read_required_secret_text(&r.node_secret_key).context("node_secret_key")?,
            gossip_dynamic_inbound_limit: required(&r.gossip_dynamic_inbound_limit)
                .and_then(|x| Ok((*x).try_into()?))
                .context("gossip_dynamic_inbound_limit")?,
            gossip_static_inbound,
            gossip_static_outbound,
            debug_page: read_optional(&r.debug_page).context("debug_page")?,
            fetch_schedule_interval: read_required(&r.fetch_schedule_interval)
                .context("fetch_schedule_interval")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            server_addr: Some(self.server_addr.encode()),
            public_addr: Some(self.public_addr.0.clone()),
            rpc_addr: self.rpc_addr.as_ref().map(TextFmt::encode),
            metrics_server_addr: self.metrics_server_addr.as_ref().map(TextFmt::encode),

            genesis: Some(self.genesis.build()),
            max_payload_size: Some(self.max_payload_size.try_into().unwrap()),
            max_tx_size: Some(self.max_tx_size.try_into().unwrap()),
            view_timeout: Some(self.view_timeout.build()),
            validator_secret_key: self.validator_key.as_ref().map(TextFmt::encode),

            node_secret_key: Some(self.node_key.encode()),
            gossip_dynamic_inbound_limit: Some(
                self.gossip_dynamic_inbound_limit.try_into().unwrap(),
            ),
            gossip_static_inbound: self
                .gossip_static_inbound
                .iter()
                .map(TextFmt::encode)
                .collect(),
            gossip_static_outbound: self
                .gossip_static_outbound
                .iter()
                .map(|(key, addr)| proto::NodeAddr {
                    key: Some(TextFmt::encode(key)),
                    addr: Some(addr.0.clone()),
                })
                .collect(),
            debug_page: self.debug_page.as_ref().map(|x| x.build()),
            fetch_schedule_interval: Some(self.fetch_schedule_interval.build()),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Tls {
    /// Cert file path
    pub cert_path: PathBuf,
    /// Key file path
    pub key_path: PathBuf,
}

/// Debug page configuration.
#[derive(Debug, PartialEq, Clone)]
pub struct DebugPage {
    /// Public Http address to listen incoming http requests.
    pub addr: SocketAddr,
}

#[derive(Debug)]
pub struct Configs {
    pub app: App,
    pub database: Option<PathBuf>,
}

impl Configs {
    pub async fn make_executor(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<(executor::Executor, EngineManagerRunner)> {
        let engine: Box<dyn EngineInterface> = if let Some(path) = &self.database {
            Box::new(engine::RocksDB::open(self.app.genesis.clone(), path).await?)
        } else {
            Box::new(in_memory::Engine::new_bounded(
                self.app.genesis.clone(),
                self.app.genesis.first_block,
                200,
            ))
        };
        let (engine_manager, runner) =
            EngineManager::new(ctx, engine, self.app.fetch_schedule_interval).await?;

        let e = executor::Executor {
            config: executor::Config {
                build_version: Some(CRATE_VERSION.parse().context("CRATE_VERSION.parse()")?),
                server_addr: self.app.server_addr,
                public_addr: self.app.public_addr.clone(),
                node_key: self.app.node_key.clone(),
                validator_key: self.app.validator_key.clone(),
                gossip_dynamic_inbound_limit: self.app.gossip_dynamic_inbound_limit,
                gossip_static_inbound: self.app.gossip_static_inbound.clone(),
                gossip_static_outbound: self.app.gossip_static_outbound.clone(),
                max_payload_size: self.app.max_payload_size,
                max_tx_size: self.app.max_tx_size,
                view_timeout: self.app.view_timeout,
                rpc: executor::RpcConfig::default(),
                debug_page: self
                    .app
                    .debug_page
                    .as_ref()
                    .map(|debug_page_config| {
                        anyhow::Ok(network::debug_page::Config {
                            addr: debug_page_config.addr,
                        })
                    })
                    .transpose()
                    .context("debug_page")?,
            },
            engine_manager,
        };
        Ok((e, runner))
    }
}
