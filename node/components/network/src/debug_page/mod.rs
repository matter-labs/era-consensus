//! Http Server to export debug information
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{atomic::Ordering, Arc},
};

use anyhow::Context as _;
use build_html::{Html, HtmlContainer, HtmlPage, Table, TableCell, TableCellType, TableRow};
use http_body_util::Full;
use human_repr::{HumanCount, HumanDuration, HumanThroughput};
use hyper::{body::Bytes, server::conn::http1, service::service_fn, Request, Response};
use hyper_util::rt::tokio::TokioIo;
use tls_listener::TlsListener;
use tokio::net::TcpListener;
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use zksync_concurrency::{ctx, net, scope};
use zksync_consensus_crypto::TextFmt as _;
use zksync_consensus_roles::{node, validator};

use crate::{gossip::Connection, MeteredStreamStats, Network};

const STYLE: &str = include_str!("style.css");

/// Http debug page configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Public Http address to listen incoming http requests.
    pub addr: SocketAddr,
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
        self.run_with_listener(ctx, listener).await
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
        *response.body_mut() = self.serve(request);
        Ok(response)
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
                self.network.gossip.cfg.server_addr
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
                    connection.stats.established.elapsed().whole_seconds() as u64,
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
                Self::human_readable_duration(stats.established.elapsed().whole_seconds() as u64),
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
