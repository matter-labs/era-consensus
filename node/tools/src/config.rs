//! Node configuration.
use crate::{proto, store};
use anyhow::Context as _;
use serde_json::{ser::Formatter, Serializer};
use std::str::FromStr;
use std::{
    collections::{HashMap, HashSet},
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use zksync_concurrency::ctx;
use zksync_consensus_bft as bft;
use zksync_consensus_crypto::{read_optional_text, read_required_text, Text, TextFmt};
use zksync_consensus_executor as executor;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{BlockStore, BlockStoreRunner, PersistentBlockStore};
use zksync_protobuf::{required, serde::Serde, ProtoFmt};

/// Ports for the nodes to listen on kubernetes pod.
const NODES_PORT: u16 = 3054;

/// Decodes a proto message from json for arbitrary ProtoFmt.
pub fn decode_json<T: serde::de::DeserializeOwned>(json: &str) -> anyhow::Result<T> {
    let mut d = serde_json::Deserializer::from_str(json);
    let p = T::deserialize(&mut d)?;
    d.end()?;
    Ok(p)
}

/// Encodes a generated proto message to json for arbitrary ProtoFmt.
pub(crate) fn encode_json<T: serde::ser::Serialize>(x: &T) -> String {
    let s = serde_json::Serializer::pretty(vec![]);
    encode_with_serializer(x, s)
}

pub(crate) fn encode_with_serializer<T: serde::ser::Serialize, F: Formatter>(
    x: &T,
    mut serializer: Serializer<Vec<u8>, F>,
) -> String {
    T::serialize(x, &mut serializer).unwrap();
    String::from_utf8(serializer.into_inner()).unwrap()
}

// pub fn encode_json<T: ProtoFmt>(x: &T) -> String {
//     let mut s = serde_json::Serializer::pretty(vec![]);
//     zksync_protobuf::serde::serialize(x, &mut s).unwrap();
//     String::from_utf8(s.into_inner()).unwrap()
// }

/// Pair of (public key, ip address) for a gossip network node.
#[derive(Debug, Clone)]
pub struct NodeAddr {
    pub key: node::PublicKey,
    pub addr: SocketAddr,
}

impl ProtoFmt for NodeAddr {
    type Proto = proto::NodeAddr;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let key = read_required_text(&r.key).context("key")?;
        let addr = read_required_text(&r.addr).context("addr")?;
        Ok(Self { addr, key })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            key: Some(TextFmt::encode(&self.key)),
            addr: Some(TextFmt::encode(&self.addr)),
        }
    }
}

/// Node configuration including executor configuration, optional validator configuration,
/// and application-specific settings (e.g. metrics scraping).
#[derive(Debug, PartialEq, Clone)]
pub struct AppConfig {
    pub server_addr: SocketAddr,
    pub public_addr: SocketAddr,
    pub metrics_server_addr: Option<SocketAddr>,

    pub validators: validator::ValidatorSet,
    pub genesis_block: validator::FinalBlock,
    pub max_payload_size: usize,

    pub gossip_dynamic_inbound_limit: usize,
    pub gossip_static_inbound: HashSet<node::PublicKey>,
    pub gossip_static_outbound: HashMap<node::PublicKey, SocketAddr>,
}

impl AppConfig {
    pub fn check_public_addr(&mut self) -> anyhow::Result<()> {
        if let Ok(public_addr) = std::env::var("PUBLIC_ADDR") {
            self.public_addr = SocketAddr::from_str(&format!("{public_addr}:{NODES_PORT}"))?;
        }
        Ok(())
    }
}

impl ProtoFmt for AppConfig {
    type Proto = proto::AppConfig;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let validators = r.validators.iter().enumerate().map(|(i, v)| {
            Text::new(v)
                .decode()
                .with_context(|| format!("validators[{i}]"))
        });
        let validators: anyhow::Result<Vec<_>> = validators.collect();
        let validators = validator::ValidatorSet::new(validators?).context("validators")?;

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
            public_addr: read_required_text(&r.public_addr).context("public_addr")?,
            metrics_server_addr: read_optional_text(&r.metrics_server_addr)
                .context("metrics_server_addr")?,

            validators,
            genesis_block: read_required_text(&r.genesis_block).context("genesis_block")?,
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
            public_addr: Some(self.public_addr.encode()),
            metrics_server_addr: self.metrics_server_addr.as_ref().map(TextFmt::encode),

            validators: self.validators.iter().map(TextFmt::encode).collect(),
            genesis_block: Some(self.genesis_block.encode()),
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
                    addr: Some(TextFmt::encode(addr)),
                })
                .collect(),
        }
    }
}

/// This struct holds the file path to each of the config files.
#[derive(Debug)]
pub struct ConfigPaths<'a> {
    /// Path to a JSON file with node configuration.
    pub app: &'a Path,
    /// Path to a validator key file.
    pub validator_key: Option<&'a Path>,
    /// Path to a node key file.
    pub node_key: &'a Path,
    /// Path to the rocksdb database.
    pub database: &'a Path,
}

pub struct Configs {
    pub app: AppConfig,
    pub validator_key: Option<validator::SecretKey>,
    pub node_key: node::SecretKey,
    pub database: PathBuf,
}

impl<'a> ConfigPaths<'a> {
    // Loads configs from the file system.
    pub fn load(self) -> anyhow::Result<Configs> {
        Ok(Configs {
            app: (|| {
                let app = fs::read_to_string(self.app).context("failed reading file")?;
                decode_json::<Serde<AppConfig>>(&app).context("failed decoding JSON")
            })()
            .with_context(|| self.app.display().to_string())?
            .0,

            validator_key: self
                .validator_key
                .as_ref()
                .map(|file| {
                    (|| {
                        let key = fs::read_to_string(file).context("failed reading file")?;
                        Text::new(&key).decode().context("failed decoding key")
                    })()
                    .with_context(|| file.display().to_string())
                })
                .transpose()?,

            node_key: (|| {
                let key = fs::read_to_string(self.node_key).context("failed reading file")?;
                Text::new(&key).decode().context("failed decoding key")
            })()
            .with_context(|| self.node_key.display().to_string())?,

            database: self.database.into(),
        })
    }
}

impl AppConfig {
    pub fn default_for(validators_amount: usize) -> (AppConfig, Vec<validator::SecretKey>) {
        // Generate the keys for all the replicas.
        let rng = &mut rand::thread_rng();

        let mut genesis = validator::GenesisSetup::empty(rng, validators_amount);
        genesis
            .next_block()
            .payload(validator::Payload(vec![]))
            .push();
        let validator_keys = genesis.keys.clone();

        (
            Self {
                server_addr: SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), NODES_PORT),
                public_addr: SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), NODES_PORT),
                metrics_server_addr: None,

                validators: genesis.validator_set(),
                genesis_block: genesis.blocks[0].clone(),
                max_payload_size: 1000000,

                gossip_dynamic_inbound_limit: 2,
                gossip_static_inbound: [].into(),
                gossip_static_outbound: [].into(),
            },
            validator_keys,
        )
    }

    pub fn with_server_addr(&mut self, server_addr: SocketAddr) -> &mut Self {
        self.server_addr = server_addr;
        self
    }

    pub fn with_public_addr(&mut self, public_addr: SocketAddr) -> &mut Self {
        self.public_addr = public_addr;
        self
    }

    pub fn with_metrics_server_addr(&mut self, metrics_server_addr: SocketAddr) -> &mut Self {
        self.metrics_server_addr = Some(metrics_server_addr);
        self
    }

    pub fn with_gossip_dynamic_inbound_limit(
        &mut self,
        gossip_dynamic_inbound_limit: usize,
    ) -> &mut Self {
        self.gossip_dynamic_inbound_limit = gossip_dynamic_inbound_limit;
        self
    }

    pub fn add_gossip_static_outbound(
        &mut self,
        key: node::PublicKey,
        addr: SocketAddr,
    ) -> &mut Self {
        self.gossip_static_outbound.insert(key, addr);
        self
    }

    pub fn add_gossip_static_inbound(&mut self, key: node::PublicKey) -> &mut Self {
        self.gossip_static_inbound.insert(key);
        self
    }

    pub fn write_to_file(self, path: &Path) -> anyhow::Result<()> {
        fs::write(path.join("config.json"), encode_json(&Serde(self))).context("fs::write()")
    }

    pub fn with_max_payload_size(&mut self, max_payload_size: usize) -> &mut Self {
        self.max_payload_size = max_payload_size;
        self
    }
}

impl Configs {
    pub async fn make_executor(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<(executor::Executor, BlockStoreRunner)> {
        let store = store::RocksDB::open(&self.database).await?;
        // Store genesis if db is empty.
        if store.is_empty().await? {
            store
                .store_next_block(ctx, &self.app.genesis_block)
                .await
                .context("store_next_block()")?;
        }
        let (block_store, runner) = BlockStore::new(ctx, Box::new(store.clone())).await?;
        let e = executor::Executor {
            config: executor::Config {
                server_addr: self.app.server_addr,
                validators: self.app.validators.clone(),
                node_key: self.node_key.clone(),
                gossip_dynamic_inbound_limit: self.app.gossip_dynamic_inbound_limit,
                gossip_static_inbound: self.app.gossip_static_inbound.clone(),
                gossip_static_outbound: self.app.gossip_static_outbound.clone(),
                max_payload_size: self.app.max_payload_size,
            },
            block_store,
            validator: self.validator_key.as_ref().map(|key| executor::Validator {
                config: executor::ValidatorConfig {
                    key: key.clone(),
                    public_addr: self.app.public_addr,
                },
                replica_store: Box::new(store),
                payload_manager: Box::new(bft::testonly::RandomPayload(self.app.max_payload_size)),
            }),
        };
        Ok((e, runner))
    }
}
