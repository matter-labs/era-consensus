//! Node configuration.
use crate::{proto, store};
use anyhow::Context as _;
use serde_json::ser::Formatter;
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
};
use zksync_concurrency::{
    ctx,
    net::{self, http::DebugCredentials},
};
use zksync_consensus_bft as bft;
use zksync_consensus_crypto::{read_optional_text, read_required_text, Text, TextFmt};
use zksync_consensus_executor as executor;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{BlockStore, BlockStoreRunner};
use zksync_protobuf::{read_required, required, ProtoFmt};

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

/// Decodes a proto message from json for arbitrary ProtoFmt.
pub fn decode_json<T: serde::de::DeserializeOwned>(json: &str) -> anyhow::Result<T> {
    let mut d = serde_json::Deserializer::from_str(json);
    let p = T::deserialize(&mut d)?;
    d.end()?;
    Ok(p)
}

/// Encodes a generated proto message to json for arbitrary ProtoFmt.
pub fn encode_json<T: serde::ser::Serialize>(x: &T) -> String {
    let s = serde_json::Serializer::pretty(vec![]);
    encode_with_serializer(x, s)
}

/// Encodes a generated proto message for arbitrary ProtoFmt with provided serializer.
pub(crate) fn encode_with_serializer<T: serde::ser::Serialize, F: Formatter>(
    x: &T,
    mut serializer: serde_json::Serializer<Vec<u8>, F>,
) -> String {
    T::serialize(x, &mut serializer).unwrap();
    String::from_utf8(serializer.into_inner()).unwrap()
}

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
pub struct AppConfig {
    pub server_addr: SocketAddr,
    pub public_addr: net::Host,
    pub rpc_addr: Option<SocketAddr>,
    pub debug_addr: Option<SocketAddr>,
    pub metrics_server_addr: Option<SocketAddr>,

    pub genesis: validator::Genesis,
    pub max_payload_size: usize,
    pub validator_key: Option<validator::SecretKey>,

    pub node_key: node::SecretKey,
    pub gossip_dynamic_inbound_limit: usize,
    pub gossip_static_inbound: HashSet<node::PublicKey>,
    pub gossip_static_outbound: HashMap<node::PublicKey, net::Host>,

    pub debug_credentials: Option<DebugCredentials>,
}

impl ProtoFmt for AppConfig {
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
        Ok(Self {
            server_addr: read_required_text(&r.server_addr).context("server_addr")?,
            public_addr: net::Host(required(&r.public_addr).context("public_addr")?.clone()),
            rpc_addr: read_optional_text(&r.rpc_addr).context("rpc_addr")?,
            metrics_server_addr: read_optional_text(&r.metrics_server_addr)
                .context("metrics_server_addr")?,

            genesis: read_required(&r.genesis).context("genesis")?,
            max_payload_size: required(&r.max_payload_size)
                .and_then(|x| Ok((*x).try_into()?))
                .context("max_payload_size")?,
            // TODO: read secret.
            validator_key: read_optional_secret_text(&r.validator_secret_key)
                .context("validator_secret_key")?,
            node_key: read_required_secret_text(&r.node_secret_key).context("node_secret_key")?,
            gossip_dynamic_inbound_limit: required(&r.gossip_dynamic_inbound_limit)
                .and_then(|x| Ok((*x).try_into()?))
                .context("gossip_dynamic_inbound_limit")?,
            gossip_static_inbound,
            gossip_static_outbound,
            debug_addr: read_optional_text(&r.debug_addr).context("debug_addr")?,
            debug_credentials: r
                .debug_credentials
                .clone()
                .map(DebugCredentials::try_from)
                .transpose()?,
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
            debug_addr: self.debug_addr.as_ref().map(TextFmt::encode),
            debug_credentials: self
                .debug_credentials
                .as_ref()
                .map(DebugCredentials::to_string),
        }
    }
}

#[derive(Debug)]
pub struct Configs {
    pub app: AppConfig,
    pub database: PathBuf,
}

impl Configs {
    pub async fn make_executor(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<(executor::Executor, BlockStoreRunner)> {
        let store = store::RocksDB::open(self.app.genesis.clone(), &self.database).await?;
        let (block_store, runner) = BlockStore::new(ctx, Box::new(store.clone())).await?;
        let e = executor::Executor {
            config: executor::Config {
                server_addr: self.app.server_addr,
                public_addr: self.app.public_addr.clone(),
                node_key: self.app.node_key.clone(),
                gossip_dynamic_inbound_limit: self.app.gossip_dynamic_inbound_limit,
                gossip_static_inbound: self.app.gossip_static_inbound.clone(),
                gossip_static_outbound: self.app.gossip_static_outbound.clone(),
                max_payload_size: self.app.max_payload_size,
                debug_page: self.app.debug_addr.map(|addr| net::http::DebugPageConfig {
                    addr,
                    credentials: self.app.debug_credentials.clone(),
                }),
            },
            block_store,
            validator: self
                .app
                .validator_key
                .as_ref()
                .map(|key| executor::Validator {
                    key: key.clone(),
                    replica_store: Box::new(store),
                    payload_manager: Box::new(bft::testonly::RandomPayload(
                        self.app.max_payload_size,
                    )),
                }),
        };
        Ok((e, runner))
    }
}
