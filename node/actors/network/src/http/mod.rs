//! Http Server to export debug information
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use build_html::{Container, ContainerType, Html, HtmlContainer, HtmlPage};
use http_body_util::Full;
use hyper::{
    body::Bytes,
    header::{self, HeaderValue},
    server::conn::http1,
    service::service_fn,
    HeaderMap, Request, Response, StatusCode,
};
use hyper_util::rt::tokio::TokioIo;
use std::{fs, io, net::SocketAddr, sync::Arc};
use tls_listener::TlsListener;
use tokio::net::TcpListener;
use tokio_rustls::{
    rustls::{
        pki_types::{CertificateDer, PrivateKeyDer},
        ServerConfig,
    },
    TlsAcceptor,
};
use zksync_concurrency::net::http::{DebugCredentials, DebugPageConfig};

use crate::{consensus, ctx, scope, Network};

/// Http Server.
pub struct Server {
    addr: SocketAddr,
    credentials: Option<DebugCredentials>,
    network: Arc<Network>,
}

impl Server {
    /// Creates a new Server
    pub fn new(config: DebugPageConfig, network: Arc<Network>) -> Server {
        Server {
            addr: config.addr,
            credentials: config.credentials,
            network,
        }
    }

    /// Runs the Server.
    pub async fn run(&self, ctx: &ctx::Ctx) -> Result<()> {
        scope::run!(ctx, |_ctx, s| async {
            let mut listener =
                TlsListener::new(tls_acceptor(), TcpListener::bind(self.addr).await?);

            // Start a loop to continuously accept incoming connections
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let io = TokioIo::new(stream);
                        s.spawn(async {
                            Ok(http1::Builder::new()
                                .serve_connection(io, service_fn(|req| self.handle(req)))
                                .await?)
                        });
                    }
                    Err(err) => {
                        if let Some(remote_addr) = err.peer_addr() {
                            tracing::error!("[client {remote_addr}] ");
                        }

                        tracing::error!("Error accepting connection: {}", err);
                    }
                }
            }
        })
        .await
    }

    async fn handle(
        &self,
        request: Request<hyper::body::Incoming>,
    ) -> Result<Response<Full<Bytes>>> {
        let mut response = Response::new(Full::default());
        match self.basic_authentication(request.headers()) {
            Ok(_) => *response.body_mut() = self.serve(request),
            Err(e) => {
                *response.status_mut() = StatusCode::UNAUTHORIZED;
                *response.body_mut() = Full::new(Bytes::from(format!("{}", e)));
                let header_value = HeaderValue::from_str(r#"Basic realm="debug""#).unwrap();
                response
                    .headers_mut()
                    .insert(header::WWW_AUTHENTICATE, header_value);
            }
        }
        Ok(response)
    }

    fn basic_authentication(&self, headers: &HeaderMap) -> Result<()> {
        self.credentials.clone().map_or(Ok(()), |credentials| {
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
            let incomming_credentials = String::from_utf8(decoded_bytes)
                .context("The decoded credential string is not valid UTF8.")?;
            if Into::<String>::into(credentials) == incomming_credentials {
                Ok(())
            } else {
                Err(anyhow::anyhow!("Invalid password."))
            }
        })
    }

    fn serve(&self, _request: Request<hyper::body::Incoming>) -> Full<Bytes> {
        let html: String = HtmlPage::new()
            .with_title("Node debug page")
            .with_header(1, "Active connections")
            .with_container(
                Container::new(ContainerType::UnorderedList)
                    .with_paragraph(format!("{:?}", "s"))
                    .with_paragraph("connection"),
            )
            .with_header(1, "Static gossip connections")
            .with_header(1, "Validator discovery data")
            .with_container(
                Container::new(ContainerType::UnorderedList)
                    .with_paragraph(format!(
                        "{:?}",
                        <Option<Arc<consensus::Network>> as Clone>::clone(&self.network.consensus)
                            .unwrap()
                            .inbound
                            .current()
                    ))
                    .with_paragraph(format!(
                        "{:?}",
                        <Option<Arc<consensus::Network>> as Clone>::clone(&self.network.consensus)
                            .unwrap()
                            .outbound
                            .current()
                    )),
            )
            .to_html_string();
        Full::new(Bytes::from(html))
    }
}

fn tls_acceptor() -> TlsAcceptor {
    let cert_der = load_certs("local.cert").expect("Invalid certificate");
    let key_der = load_private_key("local.key").expect("Invalid private key");
    Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert_der, key_der)
            .unwrap(),
    )
    .into()
}

// Load public certificate from file.
fn load_certs(filename: &str) -> Result<Vec<CertificateDer<'static>>> {
    // Open certificate file.
    let certfile =
        fs::File::open(filename).with_context(|| anyhow!("failed to open {}", filename))?;
    let mut reader = io::BufReader::new(certfile);

    // Load and return certificate.
    Ok(rustls_pemfile::certs(&mut reader)
        .map(|r| r.expect("Invalid certificate"))
        .collect())
}

// Load private key from file.
fn load_private_key(filename: &str) -> Result<PrivateKeyDer<'static>> {
    // Open keyfile.
    let keyfile =
        fs::File::open(filename).with_context(|| anyhow!("failed to open {}", filename))?;
    let mut reader = io::BufReader::new(keyfile);

    // Load and return a single private key.
    Ok(rustls_pemfile::private_key(&mut reader).map(|key| key.expect("Private key not found"))?)
}
