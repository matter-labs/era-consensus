//! Http Server to export debug information
use crate::{gossip::Connection, MeteredStreamStats, Network};
use anyhow::Context as _;
use base64::Engine;
use build_html::{Html, HtmlContainer, HtmlPage, Table, TableCell, TableCellType, TableRow};
use http_body_util::Full;
use human_repr::{HumanCount, HumanDuration, HumanThroughput};
use hyper::{
    body::Bytes,
    header::{self, HeaderValue},
    server::conn::http1,
    service::service_fn,
    HeaderMap, Request, Response, StatusCode,
};
use hyper_util::rt::tokio::TokioIo;
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{atomic::Ordering, Arc},
    time::{Duration, SystemTime},
};
use tls_listener::TlsListener;
use tokio::net::TcpListener;
use tokio_rustls::{
    rustls::{
        pki_types::{CertificateDer, PrivateKeyDer},
        ServerConfig,
    },
    server::TlsStream,
    TlsAcceptor,
};
use zksync_concurrency::{ctx, net, scope};
use zksync_consensus_crypto::TextFmt as _;
use zksync_consensus_roles::{attester, node, validator};

const STYLE: &str = include_str!("style.css");

/// TLS certificate chain with a private key.
#[derive(Debug, PartialEq)]
pub struct TlsConfig {
    /// TLS certificate chain.
    pub cert_chain: Vec<CertificateDer<'static>>,
    /// Private key for the leaf cert.
    pub private_key: PrivateKeyDer<'static>,
}

/// Credentials.
#[derive(PartialEq, Clone)]
pub struct Credentials {
    /// User for debug page
    pub user: String,
    /// Password for debug page
    /// TODO: it should be treated as a secret: zeroize, etc.
    pub password: String,
}

impl Credentials {
    fn parse(value: String) -> anyhow::Result<Self> {
        let [user, password] = value
            .split(':')
            .collect::<Vec<_>>()
            .try_into()
            .ok()
            .context("bad format")?;
        Ok(Self {
            user: user.to_string(),
            password: password.to_string(),
        })
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials").finish_non_exhaustive()
    }
}

/// Http debug page configuration.
#[derive(Debug, PartialEq)]
pub struct Config {
    /// Public Http address to listen incoming http requests.
    pub addr: SocketAddr,
    /// Debug page credentials.
    pub credentials: Option<Credentials>,
    /// TLS certificate to terminate the connections with.
    pub tls: Option<TlsConfig>,
}

/// Http Server for debug page.
pub struct Server {
    config: Config,
    network: Arc<Network>,
}

#[async_trait::async_trait]
trait Listener: 'static + Send {
    type Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin;
    async fn accept(&mut self) -> anyhow::Result<Self::Stream>;
}

#[async_trait::async_trait]
impl Listener for TcpListener {
    type Stream = tokio::net::TcpStream;
    async fn accept(&mut self) -> anyhow::Result<Self::Stream> {
        Ok(TcpListener::accept(self).await?.0)
    }
}

#[async_trait::async_trait]
impl Listener for TlsListener<TcpListener, TlsAcceptor> {
    type Stream = TlsStream<tokio::net::TcpStream>;
    async fn accept(&mut self) -> anyhow::Result<Self::Stream> {
        Ok(TlsListener::accept(self).await?.0)
    }
}

impl Server {
    /// Creates a new Server
    pub fn new(config: Config, network: Arc<Network>) -> Self {
        Self { config, network }
    }

    /// Runs the Server.
    pub async fn run(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let listener = TcpListener::bind(self.config.addr)
            .await
            .context("TcpListener::bind()")?;
        if let Some(tls) = &self.config.tls {
            let cfg = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(tls.cert_chain.clone(), tls.private_key.clone_key())
                .context("with_single_cert()")?;
            self.run_with_listener(ctx, TlsListener::new(Arc::new(cfg).into(), listener))
                .await
        } else {
            self.run_with_listener(ctx, listener).await
        }
    }

    async fn run_with_listener<L: Listener>(
        &self,
        ctx: &ctx::Ctx,
        mut listener: L,
    ) -> anyhow::Result<()> {
        // Start a watcher to shut down the server whenever ctx gets cancelled
        let graceful = hyper_util::server::graceful::GracefulShutdown::new();

        scope::run!(ctx, |ctx, s| async {
            let http = http1::Builder::new();

            // Start a loop to accept incoming connections
            while let Ok(res) = ctx.wait(listener.accept()).await {
                match res {
                    Ok(stream) => {
                        let io = TokioIo::new(stream);
                        let conn = http.serve_connection(io, service_fn(|req| self.handle(req)));
                        // watch this connection
                        let fut = graceful.watch(conn);
                        s.spawn_bg(async {
                            if let Err(e) = fut.await {
                                tracing::error!("Error serving connection: {:?}", e);
                            }
                            Ok(())
                        });
                    }
                    Err(err) => {
                        tracing::error!("Error accepting connection: {}", err);
                        continue;
                    }
                }
            }
            graceful.shutdown().await;
            Ok(())
        })
        .await
    }

    async fn handle(
        &self,
        request: Request<hyper::body::Incoming>,
    ) -> anyhow::Result<Response<Full<Bytes>>> {
        let mut response = Response::new(Full::default());
        if let Err(err) = self.authenticate(request.headers()) {
            *response.status_mut() = StatusCode::UNAUTHORIZED;
            *response.body_mut() = Full::new(Bytes::from(err.to_string()));
            let header_value = HeaderValue::from_str(r#"Basic realm="debug""#).unwrap();
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, header_value);
        }
        *response.body_mut() = self.serve(request);
        Ok(response)
    }

    fn authenticate(&self, headers: &HeaderMap) -> anyhow::Result<()> {
        let Some(want) = self.config.credentials.as_ref() else {
            return Ok(());
        };

        // The header value, if present, must be a valid UTF8 string
        let header_value = headers
            .get("Authorization")
            .context("The 'Authorization' header was missing")?
            .to_str()
            .context("The 'Authorization' header was not a valid UTF8 string.")?;
        let base64encoded_segment = header_value
            .strip_prefix("Basic ")
            .context("Unsupported authorization scheme.")?;
        let decoded_bytes = base64::engine::general_purpose::STANDARD
            .decode(base64encoded_segment)
            .context("Failed to base64-decode 'Basic' credentials.")?;
        let got = Credentials::parse(
            String::from_utf8(decoded_bytes)
                .context("The decoded credential string is not valid UTF8.")?,
        )?;
        anyhow::ensure!(want == &got, "Invalid credentials.");
        Ok(())
    }

    fn serve(&self, _request: Request<hyper::body::Incoming>) -> Full<Bytes> {
        let mut html = HtmlPage::new()
            .with_title("Node debug page")
            .with_style(STYLE);

        // Config information
        html = html
            .with_header(1, "Config")
            .with_paragraph(format!(
                "Build version: {}",
                self.network
                    .gossip
                    .cfg
                    .build_version
                    .as_ref()
                    .map_or("N/A".to_string(), |v| v.to_string())
            ))
            .with_paragraph(format!(
                "Server address: {}",
                self.network.gossip.cfg.server_addr.to_string()
            ))
            .with_paragraph(format!(
                "Public address: {}",
                self.network.gossip.cfg.public_addr.0
            ))
            .with_paragraph(format!(
                "Maximum block size: {}",
                self.network.gossip.cfg.max_block_size.human_count_bytes()
            ))
            .with_paragraph(format!(
                "Maximum block queue size: {}",
                self.network.gossip.cfg.max_block_queue_size
            ))
            .with_paragraph(format!(
                "Ping timeout: {}",
                self.network
                    .gossip
                    .cfg
                    .ping_timeout
                    .as_ref()
                    .map_or("None".to_string(), |x| x
                        .as_seconds_f32()
                        .human_duration()
                        .to_string())
            ))
            .with_paragraph(format!(
                "TCP accept rate - burst: {}, refresh: {}",
                self.network.gossip.cfg.tcp_accept_rate.burst,
                self.network
                    .gossip
                    .cfg
                    .tcp_accept_rate
                    .refresh
                    .as_seconds_f32()
                    .human_duration()
            ))
            .with_header(3, "RPC limits")
            .with_paragraph(format!(
                "push_validator_addrs rate - burst: {}, refresh: {}",
                self.network.gossip.cfg.rpc.push_validator_addrs_rate.burst,
                self.network
                    .gossip
                    .cfg
                    .rpc
                    .push_validator_addrs_rate
                    .refresh
                    .as_seconds_f32()
                    .human_duration()
            ))
            .with_paragraph(format!(
                "push_block_store_state rate - burst: {}, refresh: {}",
                self.network
                    .gossip
                    .cfg
                    .rpc
                    .push_block_store_state_rate
                    .burst,
                self.network
                    .gossip
                    .cfg
                    .rpc
                    .push_block_store_state_rate
                    .refresh
                    .as_seconds_f32()
                    .human_duration()
            ))
            .with_paragraph(format!(
                "push_batch_votes rate - burst: {}, refresh: {}",
                self.network.gossip.cfg.rpc.push_batch_votes_rate.burst,
                self.network
                    .gossip
                    .cfg
                    .rpc
                    .push_batch_votes_rate
                    .refresh
                    .as_seconds_f32()
                    .human_duration()
            ))
            .with_paragraph(format!(
                "get_block rate - burst: {}, refresh: {}",
                self.network.gossip.cfg.rpc.get_block_rate.burst,
                self.network
                    .gossip
                    .cfg
                    .rpc
                    .get_block_rate
                    .refresh
                    .as_seconds_f32()
                    .human_duration()
            ))
            .with_paragraph(format!(
                "get_block timeout: {}",
                self.network
                    .gossip
                    .cfg
                    .rpc
                    .get_block_timeout
                    .as_ref()
                    .map_or("None".to_string(), |x| x
                        .as_seconds_f32()
                        .human_duration()
                        .to_string())
            ))
            .with_paragraph(format!(
                "Consensus rate - burst: {}, refresh: {}",
                self.network.gossip.cfg.rpc.consensus_rate.burst,
                self.network
                    .gossip
                    .cfg
                    .rpc
                    .consensus_rate
                    .refresh
                    .as_seconds_f32()
                    .human_duration()
            ));

        // Gossip network
        html = html
            .with_header(1, "Gossip network")
            .with_header(2, "Config")
            .with_paragraph(format!(
                "Node public key: {}",
                self.network.gossip.cfg.gossip.key.public().encode()
            ))
            .with_paragraph(format!(
                "Dynamic incoming connections limit: {}",
                self.network.gossip.cfg.gossip.dynamic_inbound_limit
            ))
            .with_header(3, "Static incoming connections")
            .with_paragraph(Self::static_inbound_table(
                self.network.gossip.cfg.gossip.static_inbound.clone(),
            ))
            .with_header(3, "Static outbound connections")
            .with_paragraph(Self::static_outbound_table(
                self.network.gossip.cfg.gossip.static_outbound.clone(),
            ))
            .with_header(2, "Active connections")
            .with_header(3, "Incoming connections")
            .with_paragraph(Self::gossip_active_table(
                self.network.gossip.inbound.current().values(),
            ))
            .with_header(3, "Outgoing connections")
            .with_paragraph(Self::gossip_active_table(
                self.network.gossip.outbound.current().values(),
            ))
            .with_header(2, "Validator addresses")
            .with_paragraph(Self::validator_addrs_table(
                self.network.gossip.validator_addrs.current().iter(),
            ))
            .with_header(2, "Fetch queue")
            .with_paragraph(format!(
                "Blocks: {:?}",
                self.network.gossip.fetch_queue.current_blocks()
            ));

        // Attester network
        html = html
            .with_header(1, "Attester network")
            .with_paragraph(format!(
                "Node public key: {}",
                self.network
                    .gossip
                    .attestation
                    .key()
                    .clone()
                    .map_or("None".to_string(), |k| k.public().encode())
            ));

        if let Some(state) = self
            .network
            .gossip
            .attestation
            .state()
            .subscribe()
            .borrow()
            .clone()
        {
            html = html
                .with_paragraph(format!(
                    "Batch to attest:\nNumber: {}, Hash: {}, Genesis hash: {}",
                    state.info().batch_to_attest.number,
                    state.info().batch_to_attest.hash.encode(),
                    state.info().batch_to_attest.genesis.encode(),
                ))
                .with_header(2, "Committee")
                .with_paragraph(Self::attester_committee_table(
                    state.info().committee.iter(),
                ))
                .with_paragraph(format!(
                    "Total weight: {}",
                    state.info().committee.total_weight()
                ))
                .with_header(2, "Votes")
                .with_paragraph(Self::attester_votes_table(state.votes().iter()))
                .with_paragraph(format!("Total weight: {}", state.total_weight()));
        }

        // Validator network
        if let Some(consensus) = self.network.consensus.as_ref() {
            html = html
                .with_header(1, "Validator network")
                .with_paragraph(format!("Public key: {}", consensus.key.public().encode()))
                .with_header(2, "Active connections")
                .with_header(3, "Incoming connections")
                .with_paragraph(Self::validator_active_table(
                    consensus.inbound.current().iter(),
                ))
                .with_header(3, "Outgoing connections")
                .with_paragraph(Self::validator_active_table(
                    consensus.outbound.current().iter(),
                ));
        }

        Full::new(Bytes::from(html.to_html_string()))
    }

    fn static_inbound_table(connections: HashSet<node::PublicKey>) -> String {
        let mut table = Table::new().with_header_row(vec!["Public key"]);

        for key in connections {
            table.add_body_row(vec![key.encode()]);
        }

        table.to_html_string()
    }

    fn static_outbound_table(connections: HashMap<node::PublicKey, net::Host>) -> String {
        let mut table = Table::new().with_header_row(vec!["Public key", "Host address"]);

        for (key, addr) in connections {
            table.add_body_row(vec![key.encode(), addr.0]);
        }

        table.to_html_string()
    }

    fn validator_addrs_table<'a>(
        connections: impl Iterator<
            Item = (
                &'a validator::PublicKey,
                &'a Arc<validator::Signed<validator::NetAddress>>,
            ),
        >,
    ) -> String {
        let mut table = Table::new()
            .with_custom_header_row(
                TableRow::new()
                    .with_cell(TableCell::new(TableCellType::Header).with_raw("Public key"))
                    .with_cell(
                        TableCell::new(TableCellType::Header)
                            .with_attributes([("colspan", "3")])
                            .with_raw("Network address"),
                    ),
            )
            .with_header_row(vec!["", "Address", "Version", "Timestamp"]);

        for (key, addr) in connections {
            table.add_body_row(vec![
                key.encode(),
                addr.msg.addr.to_string(),
                addr.msg.version.to_string(),
                addr.msg.timestamp.to_string(),
            ])
        }

        table.to_html_string()
    }

    fn attester_committee_table<'a>(
        attesters: impl Iterator<Item = &'a attester::WeightedAttester>,
    ) -> String {
        let mut table = Table::new().with_header_row(vec!["Public key", "Weight"]);

        for attester in attesters {
            table.add_body_row(vec![attester.key.encode(), attester.weight.to_string()]);
        }

        table.to_html_string()
    }

    fn attester_votes_table<'a>(
        votes: impl Iterator<
            Item = (
                &'a attester::PublicKey,
                &'a Arc<attester::Signed<attester::Batch>>,
            ),
        >,
    ) -> String {
        let mut table = Table::new()
            .with_custom_header_row(
                TableRow::new()
                    .with_cell(TableCell::new(TableCellType::Header).with_raw("Public key"))
                    .with_cell(
                        TableCell::new(TableCellType::Header)
                            .with_attributes([("colspan", "3")])
                            .with_raw("Batch"),
                    ),
            )
            .with_header_row(vec!["", "Number", "Hash", "Genesis hash"]);

        for (key, batch) in votes {
            table.add_body_row(vec![
                key.encode(),
                batch.msg.number.to_string(),
                batch.msg.hash.encode(),
                batch.msg.genesis.encode(),
            ])
        }

        table.to_html_string()
    }

    fn gossip_active_table<'a>(connections: impl Iterator<Item = &'a Arc<Connection>>) -> String {
        let mut table = Table::new().with_header_row(vec![
            "Public key",
            "Address",
            "Build version",
            "Download speed",
            "Download total",
            "Upload speed",
            "Upload total",
            "Age",
        ]);

        for connection in connections {
            table.add_body_row(vec![
                connection.key.encode(),
                connection.stats.peer_addr.to_string(),
                connection
                    .build_version
                    .as_ref()
                    .map_or("N/A".to_string(), |v| v.to_string()),
                connection
                    .stats
                    .received_throughput()
                    .human_throughput_bytes()
                    .to_string(),
                connection
                    .stats
                    .received
                    .load(Ordering::Relaxed)
                    .human_count_bytes()
                    .to_string(),
                connection
                    .stats
                    .sent_throughput()
                    .human_throughput_bytes()
                    .to_string(),
                connection
                    .stats
                    .sent
                    .load(Ordering::Relaxed)
                    .human_count_bytes()
                    .to_string(),
                Self::human_readable_duration(
                    SystemTime::now()
                        .duration_since(connection.stats.established)
                        .unwrap_or_default()
                        .as_secs(),
                ),
            ])
        }

        table.to_html_string()
    }

    fn validator_active_table<'a>(
        connections: impl Iterator<Item = (&'a validator::PublicKey, &'a Arc<MeteredStreamStats>)>,
    ) -> String {
        let mut table = Table::new().with_header_row(vec![
            "Public key",
            "Address",
            "Download speed",
            "Download total",
            "Upload speed",
            "Upload total",
            "Age",
        ]);

        for (key, stats) in connections {
            table.add_body_row(vec![
                key.encode(),
                stats.peer_addr.to_string(),
                stats
                    .received_throughput()
                    .human_throughput_bytes()
                    .to_string(),
                stats
                    .received
                    .load(Ordering::Relaxed)
                    .human_count_bytes()
                    .to_string(),
                stats.sent_throughput().human_throughput_bytes().to_string(),
                stats
                    .sent
                    .load(Ordering::Relaxed)
                    .human_count_bytes()
                    .to_string(),
                Self::human_readable_duration(
                    SystemTime::now()
                        .duration_since(stats.established)
                        .unwrap_or_default()
                        .as_secs(),
                ),
            ])
        }

        table.to_html_string()
    }

    /// Returns human readable duration. We use this function instead of `Duration::human_duration` because
    /// we want to show days as well.
    fn human_readable_duration(seconds: u64) -> String {
        let days = seconds / 86400;
        let hours = (seconds % 86400) / 3600;
        let minutes = (seconds % 3600) / 60;
        let seconds = seconds % 60;

        let mut components = Vec::new();

        if days > 0 {
            components.push(format!("{}d", days));
        }
        if hours > 0 {
            components.push(format!("{}h", hours));
        }
        if minutes > 0 {
            components.push(format!("{}m", minutes));
        }

        components.push(format!("{}s", seconds));
        components.join(" ")
    }
}
