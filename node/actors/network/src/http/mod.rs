//! Http Server to export debug information
use crate::{MeteredStreamStats, Network};
use zksync_consensus_crypto::TextFmt as _;
use anyhow::Context as _;
use base64::Engine;
use build_html::{Html, HtmlContainer, HtmlPage, Table, TableCell, TableCellType, TableRow};
use http_body_util::Full;
use hyper::{
    body::Bytes,
    header::{self, HeaderValue},
    server::conn::http1,
    service::service_fn,
    HeaderMap, Request, Response, StatusCode,
};
use std::sync::atomic::Ordering;
use hyper_util::rt::tokio::TokioIo;
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tls_listener::TlsListener;
use tokio::net::TcpListener;
use tokio_rustls::{
    rustls::{
        pki_types::{CertificateDer, PrivateKeyDer},
        ServerConfig,
    },
    TlsAcceptor,
};
use zksync_concurrency::{ctx, scope};
use zksync_consensus_utils::debug_page;

const STYLE: &str = include_str!("style.css");

/// Http debug page configuration.
#[derive(Debug, PartialEq)]
pub struct DebugPageConfig {
    /// Public Http address to listen incoming http requests.
    pub addr: SocketAddr,
    /// Debug page credentials.
    pub credentials: Option<debug_page::Credentials>,
    /// Cert file path
    pub certs: Vec<CertificateDer<'static>>,
    /// Key file path
    pub private_key: PrivateKeyDer<'static>,
}

/// Http Server for debug page.
pub struct DebugPageServer {
    config: DebugPageConfig,
    network: Arc<Network>,
}

impl DebugPageServer {
    /// Creates a new Server
    pub fn new(config: DebugPageConfig, network: Arc<Network>) -> DebugPageServer {
        DebugPageServer { config, network }
    }

    /// Runs the Server.
    pub async fn run(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        // Start a watcher to shut down the server whenever ctx gets cancelled
        let graceful = hyper_util::server::graceful::GracefulShutdown::new();

        scope::run!(ctx, |ctx, s| async {
            let mut listener = TlsListener::new(
                self.tls_acceptor(),
                TcpListener::bind(self.config.addr).await?,
            );

            let http = http1::Builder::new();

            // Start a loop to accept incoming connections
            while let Ok(res) = ctx.wait(listener.accept()).await {
                match res {
                    Ok((stream, _)) => {
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
                        if let Some(remote_addr) = err.peer_addr() {
                            tracing::error!("[client {remote_addr}] ");
                        }

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
        match self.basic_authentication(request.headers()) {
            Ok(_) => *response.body_mut() = self.serve(request),
            Err(e) => {
                *response.status_mut() = StatusCode::UNAUTHORIZED;
                *response.body_mut() = Full::new(Bytes::from(e.to_string()));
                let header_value = HeaderValue::from_str(r#"Basic realm="debug""#).unwrap();
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, header_value);
            }
        }
        Ok(response)
    }

    fn basic_authentication(&self, headers: &HeaderMap) -> anyhow::Result<()> {
        self.config
            .credentials
            .clone()
            .map_or(Ok(()), |credentials| {
                // The header value, if present, must be a valid UTF8 string
                let header_value = headers
                    .get("Authorization")
                    .context("The 'Authorization' header was missing")?
                    .to_str()
                    .context("The 'Authorization' header was not a valid UTF8 string.")?;
                let base64encoded_segment = header_value
                    .strip_prefix("Basic ")
                    .context("The authorization scheme was not 'Basic'.")?;
                let decoded_bytes = base64::engine::general_purpose::STANDARD
                    .decode(base64encoded_segment)
                    .context("Failed to base64-decode 'Basic' credentials.")?;
                let incoming_credentials = debug_page::Credentials::try_from(
                    String::from_utf8(decoded_bytes)
                        .context("The decoded credential string is not valid UTF8.")?,
                )?;
                if credentials != incoming_credentials {
                    anyhow::bail!("Invalid password.")
                }
                Ok(())
            })
    }

    fn serve(&self, _request: Request<hyper::body::Incoming>) -> Full<Bytes> {
        let mut html = HtmlPage::new()
            .with_title("Node debug page")
            .with_style(STYLE)
            .with_header(1, "Active connections");
        if let Some(consensus) = self.network.consensus.as_ref() {
            html = html
                .with_header(2, "Validator network")
                .with_header(3, "Incoming connections")
                .with_paragraph(self.connections_html(consensus.inbound.current().iter().map(|(k,v)|(k.encode(),v))))
                .with_header(3, "Outgoing connections")
                .with_paragraph(self.connections_html(consensus.outbound.current().iter().map(|(k,v)|(k.encode(),v))));
        }
        html = html
            .with_header(2, "Gossip network")
            .with_header(3, "Incoming connections")
            .with_paragraph(self.connections_html(self.network.gossip.inbound.current().iter().map(|(k,v)|(k.encode(),&v.stats))))
            .with_header(3, "Outgoing connections")
            .with_paragraph(self.connections_html(self.network.gossip.outbound.current().iter().map(|(k,v)|(k.encode(),&v.stats))));
        Full::new(Bytes::from(html.to_html_string()))
    }

    fn connections_html<'a>(&self, connections: impl Iterator<Item=(String,&'a Arc<MeteredStreamStats>)>) -> String {
        let mut table = Table::new()
            .with_custom_header_row(
                TableRow::new()
                    .with_cell(TableCell::new(TableCellType::Header).with_raw("Public key"))
                    .with_cell(TableCell::new(TableCellType::Header).with_raw("Address"))
                    .with_cell(
                        TableCell::new(TableCellType::Header)
                            .with_attributes([("colspan", "2")])
                            .with_raw("Incoming"),
                    )
                    .with_cell(
                        TableCell::new(TableCellType::Header)
                            .with_attributes([("colspan", "2")])
                            .with_raw("Outgoing"),
                    )
                    .with_cell(TableCell::new(TableCellType::Header).with_raw("Age")),
            )
            .with_header_row(vec!["", "", "size", "bandwidth", "size", "bandwidth", ""]);
        for (key, values) in connections {
            let age = SystemTime::now()
                .duration_since(values.established)
                .ok()
                .unwrap_or_else(|| Duration::new(1, 0))
                .max(Duration::new(1, 0)); // Ensure Duration is not 0 to prevent division by zero
            let received = values.received.load(Ordering::Relaxed);
            let sent = values.sent.load(Ordering::Relaxed);
            table.add_body_row(vec![
                Self::shorten(key),
                values.peer_addr.to_string(),
                bytesize::to_string(received, false),
                bytesize::to_string(received / age.as_secs(), false) + "/s",
                bytesize::to_string(sent, false),
                bytesize::to_string(sent / age.as_secs(), false) + "/s",
                format!("{}s", age.as_secs()),
            ])
        }
        table.to_html_string()
    }

    fn shorten(key: String) -> String {
        key.strip_prefix("validator:public:bls12_381:")
            .or(key.strip_prefix("node:public:ed25519:"))
            .map_or("-".to_string(), |key| {
                let len = key.len();
                format!("{}...{}", &key[..10], &key[len - 11..len])
            })
    }

    fn tls_acceptor(&self) -> TlsAcceptor {
        let cert_der = self.config.certs.clone();
        let key_der = self.config.private_key.clone_key();
        Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(cert_der, key_der)
                .unwrap(),
        )
        .into()
    }
}
