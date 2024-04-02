//! Node configuration.
use crate::{proto, store};
use anyhow::Context as _;
use serde_json::{ser::Formatter, Serializer};
use std::{
    collections::{HashMap, HashSet},
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use zksync_concurrency::{ctx, net};
use zksync_consensus_bft as bft;
use zksync_consensus_crypto::{read_optional_text, read_required_text, Text, TextFmt};
use zksync_consensus_executor as executor;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{BlockStore, BlockStoreRunner};
use zksync_protobuf::{read_required, required, serde::Serde, ProtoFmt};

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
    mut serializer: Serializer<Vec<u8>, F>,
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
    pub metrics_server_addr: Option<SocketAddr>,

    pub genesis: validator::Genesis,
    pub max_payload_size: usize,

    pub gossip_dynamic_inbound_limit: usize,
    pub gossip_static_inbound: HashSet<node::PublicKey>,
    pub gossip_static_outbound: HashMap<node::PublicKey, net::Host>,
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
            metrics_server_addr: read_optional_text(&r.metrics_server_addr)
                .context("metrics_server_addr")?,

            genesis: read_required(&r.genesis).context("genesis")?,
            max_payload_size: required(&r.max_payload_size)
                .and_then(|x| Ok((*x).try_into()?))
                .context("max_payload_size")?,

            gossip_dynamic_inbound_limit: required(&r.gossip_dynamic_inbound_limit)
                .and_then(|x| Ok((*x).try_into()?))
                .context("gossip_dynamic_inbound_limit")?,
            gossip_static_inbound,
            gossip_static_outbound,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            server_addr: Some(self.server_addr.encode()),
            public_addr: Some(self.public_addr.0.clone()),
            metrics_server_addr: self.metrics_server_addr.as_ref().map(TextFmt::encode),

            genesis: Some(self.genesis.build()),
            max_payload_size: Some(self.max_payload_size.try_into().unwrap()),

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
        }
    }
}

/// Configuration information.
#[derive(Debug)]
pub struct ConfigArgs<'a> {
    /// Node configuration.
    pub config_args: ConfigSource<'a>,
    /// Path to the rocksdb database.
    pub database: &'a Path,
}

#[derive(Debug)]
pub enum ConfigSource<'a> {
    CliConfig {
        /// Node configuration from command line.
        config: AppConfig,
        /// Node key as a string.
        node_key: node::SecretKey,
        /// Validator key as a string.
        validator_key: Option<validator::SecretKey>,
    },
    PathConfig {
        /// Path to a JSON file with node configuration.
        config_file: &'a Path,
        /// Path to a validator key file.
        validator_key_file: &'a Path,
        /// Path to a node key file.
        node_key_file: &'a Path,
    },
}

pub struct Configs {
    pub app: AppConfig,
    pub validator_key: Option<validator::SecretKey>,
    pub node_key: node::SecretKey,
    pub database: PathBuf,
}

impl<'a> ConfigArgs<'a> {
    // Loads configs from the file system.
    pub fn load(self) -> anyhow::Result<Configs> {
        match self.config_args {
            ConfigSource::CliConfig {
                config,
                node_key,
                validator_key,
            } => Ok(Configs {
                app: config.clone(),
                validator_key: validator_key.clone(),
                node_key: node_key.clone(),
                database: self.database.into(),
            }),
            ConfigSource::PathConfig {
                config_file,
                validator_key_file,
                node_key_file,
            } => Ok(Configs {
                app: (|| {
                    let app = fs::read_to_string(config_file).context("failed reading file")?;
                    decode_json::<Serde<AppConfig>>(&app).context("failed decoding JSON")
                })()
                .with_context(|| config_file.display().to_string())?
                .0,

                validator_key: fs::read_to_string(validator_key_file)
                    .ok()
                    .map(|value| Text::new(&value).decode().context("failed decoding key"))
                    .transpose()
                    .with_context(|| validator_key_file.display().to_string())?,

                node_key: (|| {
                    let key = fs::read_to_string(node_key_file).context("failed reading file")?;
                    Text::new(&key).decode().context("failed decoding key")
                })()
                .with_context(|| node_key_file.display().to_string())?,

                database: self.database.into(),
            }),
        }
    }
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
                node_key: self.node_key.clone(),
                gossip_dynamic_inbound_limit: self.app.gossip_dynamic_inbound_limit,
                gossip_static_inbound: self.app.gossip_static_inbound.clone(),
                gossip_static_outbound: self.app.gossip_static_outbound.clone(),
                max_payload_size: self.app.max_payload_size,
            },
            block_store,
            validator: self.validator_key.as_ref().map(|key| executor::Validator {
                key: key.clone(),
                replica_store: Box::new(store),
                payload_manager: Box::new(bft::testonly::RandomPayload(self.app.max_payload_size)),
            }),
        };
        Ok((e, runner))
    }
}
