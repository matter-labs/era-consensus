//! Node configuration.
use anyhow::Context as _;
use crate::proto;
use std::collections::{HashMap,HashSet};
use std::{fs, path::{Path,PathBuf}, sync::Arc};
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{read_required_text, read_optional_text, Text, TextFmt};
use zksync_consensus_executor as executor;
use zksync_consensus_bft as bft;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::{required, ProtoFmt};
use zksync_consensus_storage::{RocksdbStorage};

/// Decodes a proto message from json for arbitrary ProtoFmt.
fn decode_json<T: ProtoFmt>(json: &str) -> anyhow::Result<T> {
    let mut d = serde_json::Deserializer::from_str(json);
    let p: T = zksync_protobuf::serde::deserialize(&mut d)?;
    d.end()?;
    Ok(p)
}

/// Node configuration including executor configuration, optional validator configuration,
/// and application-specific settings (e.g. metrics scraping).
#[derive(Debug)]
pub struct AppConfig {
    pub server_addr: std::net::SocketAddr,
    pub public_addr: std::net::SocketAddr,
    pub metrics_server_addr: Option<std::net::SocketAddr>,

    pub validator_key: Option<validator::PublicKey>,
    pub validators: validator::ValidatorSet,
    pub genesis_block: validator::FinalBlock,

    pub node_key: node::PublicKey,
    pub gossip_dynamic_inbound_limit: u64,
    pub gossip_static_inbound: HashSet<node::PublicKey>,
    pub gossip_static_outbound: HashMap<node::PublicKey,std::net::SocketAddr>,
}

impl ProtoFmt for AppConfig {
    type Proto = proto::AppConfig;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let validators = r.validators.iter().enumerate().map(|(i, v)| {
            Text::new(v).decode().with_context(|| format!("validators[{i}]"))
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
            let key = read_required_text(&e.key)
                .with_context(|| format!("gossip_static_outbound[{i}].key"))?;
            let addr = read_required_text(&e.addr)
                .with_context(|| format!("gossip_static_outbound[{i}].addr"))?;
            gossip_static_outbound.insert(key, addr);
        }
        Ok(Self {
            server_addr: read_required_text(&r.server_addr).context("server_addr")?,
            public_addr: read_required_text(&r.public_addr).context("public_addr")?,
            metrics_server_addr: read_optional_text(&r.metrics_server_addr).context("metrics_server_addr")?,
        
            validator_key: read_optional_text(&r.validator_key).context("validator_key")?,
            validators,
            genesis_block: read_required_text(&r.genesis_block).context("genesis_block")?,
        
            node_key: read_required_text(&r.node_key).context("node_key")?,
            gossip_dynamic_inbound_limit: *required(&r.gossip_dynamic_inbound_limit).context("gossip_dynamic_inbound_limit")?,
            gossip_static_inbound,
            gossip_static_outbound,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            server_addr: Some(self.server_addr.encode()),
            public_addr: Some(self.public_addr.encode()),
            metrics_server_addr: self.metrics_server_addr.as_ref().map(TextFmt::encode),

            validator_key: self.validator_key.as_ref().map(TextFmt::encode),
            validators: self.validators.iter().map(TextFmt::encode).collect(),
            genesis_block: Some(self.genesis_block.encode()),

            node_key: Some(self.node_key.encode()),
            gossip_dynamic_inbound_limit: Some(self.gossip_dynamic_inbound_limit),
            gossip_static_inbound: self.gossip_static_inbound.iter().map(TextFmt::encode).collect(),
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
            app: (||{
                let app = fs::read_to_string(self.app).context("failed reading file")?;
                decode_json(&app).context("failed decoding JSON")
            })().with_context(||self.app.display().to_string())?,

            validator_key: self.validator_key.as_ref().map(|file| {
                (||{
                    let key = fs::read_to_string(file).context("failed reading file")?;
                    Text::new(&key).decode().context("failed decoding key")
                })().with_context(||file.display().to_string())
            }).transpose()?,
            
            node_key: (||{
                let key = fs::read_to_string(self.node_key).context("failed reading file")?;
                Text::new(&key).decode().context("failed decoding key")
            })().with_context(||self.node_key.display().to_string())?,

            database: self.database.into(),
        })
    }
}

impl Configs {
    pub async fn into_executor(self, ctx: &ctx::Ctx) -> anyhow::Result<executor::Executor> {
        anyhow::ensure!(
            self.app.node_key==self.node_key.public(),
            "node secret key has to match the node public key in the app config",
        );
        anyhow::ensure!(
            self.app.validator_key==self.validator_key.as_ref().map(|k|k.public()),
            "validator secret key has to match the validator public key in the app config",
        );
        let storage = RocksdbStorage::new(
            ctx,
            &self.app.genesis_block,
            &self.database,
        ).await.context("RocksdbStorage::new()")?;
        let storage = Arc::new(storage);
        Ok(executor::Executor {
            config: executor::Config {
                server_addr: self.app.server_addr,
                validators: self.app.validators,
                node_key: self.node_key,
                gossip_dynamic_inbound_limit: self.app.gossip_dynamic_inbound_limit,
                gossip_static_inbound: self.app.gossip_static_inbound,
                gossip_static_outbound: self.app.gossip_static_outbound,
            },
            storage: storage.clone(),
            validator: self.validator_key.map(|key| executor::Validator {
                config: executor::ValidatorConfig { key, public_addr: self.app.public_addr },
                replica_state_store: storage,
                payload_source: Arc::new(bft::testonly::RandomPayloadSource),
            })
        })
    }
}
