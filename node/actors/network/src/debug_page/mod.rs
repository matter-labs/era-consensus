//! Http Server to export debug information
use crate::{MeteredStreamStats, Network};
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
use hyper_util::rt::tokio::TokioIo;
use std::{
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
use zksync_concurrency::{ctx, scope};
use zksync_consensus_crypto::TextFmt as _;

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
                .context("with_signle_cert()")?;
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
            .with_style(STYLE)
            .with_header(1, "Active connections");
        if let Some(consensus) = self.network.consensus.as_ref() {
            html = html
                .with_header(2, "Validator network")
                .with_header(3, "Incoming connections")
                .with_paragraph(
                    self.connections_html(
                        consensus
                            .inbound
                            .current()
                            .iter()
                            .map(|(k, v)| (k.encode(), v, None)),
                    ),
                )
                .with_header(3, "Outgoing connections")
                .with_paragraph(
                    self.connections_html(
                        consensus
                            .outbound
                            .current()
                            .iter()
                            .map(|(k, v)| (k.encode(), v, None)),
                    ),
                );
        }
        html = html
            .with_header(2, "Gossip network")
            .with_header(3, "Incoming connections")
            .with_paragraph(
                self.connections_html(
                    self.network
                        .gossip
                        .inbound
                        .current()
                        .values()
                        .map(|c| (c.key.encode(), &c.stats, c.build_version.clone())),
                ),
            )
            .with_header(3, "Outgoing connections")
            .with_paragraph(
                self.connections_html(
                    self.network
                        .gossip
                        .outbound
                        .current()
                        .values()
                        .map(|c| (c.key.encode(), &c.stats, c.build_version.clone())),
                ),
            );
        Full::new(Bytes::from(html.to_html_string()))
    }

    fn connections_html<'a>(
        &self,
        connections: impl Iterator<
            Item = (String, &'a Arc<MeteredStreamStats>, Option<semver::Version>),
        >,
    ) -> String {
        let mut table = Table::new()
            .with_custom_header_row(
                TableRow::new()
                    .with_cell(TableCell::new(TableCellType::Header).with_raw("Public key"))
                    .with_cell(TableCell::new(TableCellType::Header).with_raw("Address"))
                    .with_cell(TableCell::new(TableCellType::Header).with_raw("Build version"))
                    .with_cell(
                        TableCell::new(TableCellType::Header)
                            .with_attributes([("colspan", "2")])
                            .with_raw("received [B]"),
                    )
                    .with_cell(
                        TableCell::new(TableCellType::Header)
                            .with_attributes([("colspan", "2")])
                            .with_raw("sent [B]"),
                    )
                    .with_cell(TableCell::new(TableCellType::Header).with_raw("Age")),
            )
            .with_header_row(vec!["", "", "", "total", "avg", "total", "avg", ""]);
        for (key, stats, build_version) in connections {
            let age = SystemTime::now()
                .duration_since(stats.established)
                .ok()
                .unwrap_or_else(|| Duration::new(1, 0))
                .max(Duration::new(1, 0)); // Ensure Duration is not 0 to prevent division by zero
            let received = stats.received.load(Ordering::Relaxed);
            let sent = stats.sent.load(Ordering::Relaxed);
            table.add_body_row(vec![
                Self::shorten(key),
                stats.peer_addr.to_string(),
                build_version.map(|v| v.to_string()).unwrap_or_default(),
                bytesize::to_string(received, false),
                // TODO: this is not useful - we should display avg from the last ~1min instead.
                bytesize::to_string(received / age.as_secs(), false) + "/s",
                bytesize::to_string(sent, false),
                bytesize::to_string(sent / age.as_secs(), false) + "/s",
                // TODO: this is not a human-friendly format, use days + hours + minutes + seconds,
                // or similar.
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
}
