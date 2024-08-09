//! Node configuration.
use crate::{proto, store};
use anyhow::{anyhow, Context as _};
use serde_json::ser::Formatter;
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use zksync_concurrency::{ctx, net, time};
use zksync_consensus_bft as bft;
use zksync_consensus_crypto::{read_optional_text, read_required_text, Text, TextFmt};
use zksync_consensus_executor::{self as executor, attestation};
use zksync_consensus_network::http;
use zksync_consensus_roles::{attester, node, validator};
use zksync_consensus_storage::testonly::{TestMemoryStorage, TestMemoryStorageRunner};
use zksync_consensus_utils::debug_page;
use zksync_protobuf::{kB, read_required, required, ProtoFmt};

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
    pub metrics_server_addr: Option<SocketAddr>,

    pub genesis: validator::Genesis,
    pub max_payload_size: usize,
    pub max_batch_size: usize,
    pub validator_key: Option<validator::SecretKey>,
    pub attester_key: Option<attester::SecretKey>,

    pub node_key: node::SecretKey,
    pub gossip_dynamic_inbound_limit: usize,
    pub gossip_static_inbound: HashSet<node::PublicKey>,
    pub gossip_static_outbound: HashMap<node::PublicKey, net::Host>,

    pub debug_page: Option<BasicDebugPageConfig>,
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

        let max_payload_size = required(&r.max_payload_size)
            .and_then(|x| Ok((*x).try_into()?))
            .context("max_payload_size")?;

        let max_batch_size = match &r.max_batch_size {
            Some(x) => (*x).try_into().context("max_payload_size")?,
            // Arbitrary estimate of 100 blocks  + 1kB for the merkle proof.
            // NOTE: this test node currently doesn't implement batches at all.
            // Once it does, do the estimates again.
            None => max_payload_size * 100 + kB,
        };

        Ok(Self {
            server_addr: read_required_text(&r.server_addr).context("server_addr")?,
            public_addr: net::Host(required(&r.public_addr).context("public_addr")?.clone()),
            rpc_addr: read_optional_text(&r.rpc_addr).context("rpc_addr")?,
            metrics_server_addr: read_optional_text(&r.metrics_server_addr)
                .context("metrics_server_addr")?,

            genesis: read_required(&r.genesis).context("genesis")?,
            max_payload_size,
            max_batch_size,
            // TODO: read secret.
            validator_key: read_optional_secret_text(&r.validator_secret_key)
                .context("validator_secret_key")?,
            attester_key: read_optional_secret_text(&r.attester_secret_key)
                .context("attester_secret_key")?,

            node_key: read_required_secret_text(&r.node_secret_key).context("node_secret_key")?,
            gossip_dynamic_inbound_limit: required(&r.gossip_dynamic_inbound_limit)
                .and_then(|x| Ok((*x).try_into()?))
                .context("gossip_dynamic_inbound_limit")?,
            gossip_static_inbound,
            gossip_static_outbound,
            debug_page: match read_optional_text(&r.debug_addr).context("debug_addr")? {
                Some(addr) => Some(BasicDebugPageConfig {
                    addr,
                    credentials: r
                        .debug_credentials
                        .clone()
                        .map(debug_page::Credentials::try_from)
                        .transpose()?,
                    cert_path: read_optional_text(&r.debug_cert_path)
                        .context("debug_cert_path")?
                        // default to 'cert.pem' if cert_path is missing
                        .unwrap_or(PathBuf::from("cert.pem")),
                    key_path: read_optional_text(&r.debug_key_path)
                        .context("debug_key_path")?
                        // default to 'key.pem' if key_path is missing
                        .unwrap_or(PathBuf::from("key.pem")),
                }),
                _ => None,
            },
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
            max_batch_size: Some(self.max_batch_size.try_into().unwrap()),
            validator_secret_key: self.validator_key.as_ref().map(TextFmt::encode),
            attester_secret_key: self.attester_key.as_ref().map(TextFmt::encode),

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
            debug_addr: self.debug_page.as_ref().map(|config| config.addr.encode()),
            debug_credentials: self.debug_page.as_ref().map(|config| {
                config
                    .credentials
                    .clone()
                    .map(debug_page::Credentials::into)
                    .unwrap()
            }),
            debug_cert_path: self
                .debug_page
                .as_ref()
                .map(|config| config.cert_path.encode()),
            debug_key_path: self
                .debug_page
                .as_ref()
                .map(|config| config.key_path.encode()),
        }
    }
}

/// Basic http debug page configuration.
/// Paths will be converted to actual cert and private key
/// on zksync_consensus_network::http::DebugPageConfig struct
#[derive(Debug, PartialEq, Clone)]
pub struct BasicDebugPageConfig {
    /// Public Http address to listen incoming http requests.
    pub addr: SocketAddr,
    /// Debug page credentials.
    pub credentials: Option<debug_page::Credentials>,
    /// Cert file path
    pub cert_path: PathBuf,
    /// Key file path
    pub key_path: PathBuf,
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
    ) -> ctx::Result<(executor::Executor, TestMemoryStorageRunner)> {
        let replica_store = store::RocksDB::open(self.app.genesis.clone(), &self.database).await?;
        let store = TestMemoryStorage::new(ctx, &self.app.genesis).await;

        // We don't have an API to poll in this setup, we can only create a local store based attestation client.
        let attestation_state =
            Arc::new(attestation::StateWatch::new(self.app.attester_key.clone()));
        let runner = store.runner;

        let e = executor::Executor {
            config: executor::Config {
                server_addr: self.app.server_addr,
                public_addr: self.app.public_addr.clone(),
                node_key: self.app.node_key.clone(),
                gossip_dynamic_inbound_limit: self.app.gossip_dynamic_inbound_limit,
                gossip_static_inbound: self.app.gossip_static_inbound.clone(),
                gossip_static_outbound: self.app.gossip_static_outbound.clone(),
                max_payload_size: self.app.max_payload_size,
                max_batch_size: self.app.max_batch_size,
                rpc: executor::RpcConfig::default(),
                debug_page: self.app.debug_page.as_ref().map(|debug_page_config| {
                    http::DebugPageConfig {
                        addr: debug_page_config.addr,
                        credentials: debug_page_config.credentials.clone(),
                        certs: load_certs(&debug_page_config.cert_path)
                            .expect("Could not obtain certs for debug page"),
                        private_key: load_private_key(&debug_page_config.key_path)
                            .expect("Could not obtain private key for debug page"),
                    }
                }),
                batch_poll_interval: time::Duration::seconds(1),
            },
            block_store: store.blocks,
            batch_store: store.batches,
            validator: self
                .app
                .validator_key
                .as_ref()
                .map(|key| executor::Validator {
                    key: key.clone(),
                    replica_store: Box::new(replica_store),
                    payload_manager: Box::new(bft::testonly::RandomPayload(
                        self.app.max_payload_size,
                    )),
                }),
            attestation_state,
        };
        Ok((e, runner))
    }
}

/// Load public certificate from file.
fn load_certs(path: &PathBuf) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    // Open certificate file.
    let certfile = fs::File::open(path).with_context(|| anyhow!("failed to open {:?}", path))?;
    let mut reader = io::BufReader::new(certfile);

    // Load and return certificate.
    Ok(rustls_pemfile::certs(&mut reader)
        .map(|r| r.expect("Invalid certificate"))
        .collect())
}

/// Load private key from file.
fn load_private_key(path: &PathBuf) -> anyhow::Result<PrivateKeyDer<'static>> {
    // Open keyfile.
    let keyfile = fs::File::open(path).with_context(|| anyhow!("failed to open {:?}", path))?;
    let mut reader = io::BufReader::new(keyfile);

    // Load and return a single private key.
    Ok(rustls_pemfile::private_key(&mut reader).map(|key| key.expect("Private key not found"))?)
}
