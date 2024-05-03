//! Http Server to export debug information
use std::convert::Infallible;
use std::net::SocketAddr;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::tokio::TokioIo;
use tls_listener::TlsListener;
use tokio::net::TcpListener;
use zksync_concurrency::{ctx, scope};

mod tls_config;
use tls_config::tls_acceptor;

/// Http Server.
pub struct Server {
    
}

impl Server {
    
    /// Creates a new Server
    pub fn new() -> Server {
        Server {}
    }
    

    /// Runs the Server.
    pub async fn run(
        self,
        ctx: &ctx::Ctx,
    ) -> anyhow::Result<()> {
        scope::run!(ctx, |_ctx, s| async {
            let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

            let mut listener = TlsListener::new(tls_acceptor(), TcpListener::bind(addr).await?);
            // let listener = TcpListener::bind(addr).await?;
        
            // Start a loop to continuously accept incoming connections
            loop {
                let (stream, _) = listener.accept().await?;
                let io = TokioIo::new(stream);

                // Spawn a task to serve multiple connections concurrently
                s.spawn(async move {
                    Ok(http1::Builder::new()
                        .serve_connection(io, service_fn(hello))
                        .await?)});
            }
        })
        .await
    }
}

async fn hello(_: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
}

