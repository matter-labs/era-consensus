use super::methods::{health_check::HealthCheck, peers::PeersInfo, RPCMethod};
use jsonrpsee::{
    server::{middleware::http::ProxyGetRequestLayer, RpcModule, Server},
    types::{error::ErrorCode, ErrorObject},
    MethodResponse,
};
use std::net::SocketAddr;
use zksync_concurrency::{ctx, scope};

/// RPC server.
pub struct RPCServer {
    /// IP address to bind to.
    ip_address: SocketAddr,
}

impl RPCServer {
    pub fn new(ip_address: SocketAddr) -> Self {
        Self { ip_address }
    }

    /// Runs the RPC server.
    pub async fn run(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        // Custom tower service to handle the RPC requests
        let service_builder = tower::ServiceBuilder::new()
            // Proxy `GET /<path>` requests to internal methods.
            .layer(ProxyGetRequestLayer::new(
                HealthCheck::path(),
                HealthCheck::method(),
            )?)
            .layer(ProxyGetRequestLayer::new(
                PeersInfo::path(),
                PeersInfo::method(),
            )?);

        let server = Server::builder()
            .set_http_middleware(service_builder)
            .build(self.ip_address)
            .await?;

        let mut module = RpcModule::new(());
        module.register_method(HealthCheck::method(), |params, _| {
            HealthCheck::callback(params)
        })?;
        module.register_method(PeersInfo::method(), |params, _| PeersInfo::callback(params))?;

        let handle = server.start(module);
        scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(async {
                ctx.canceled().await;
                // Ignore `AlreadyStoppedError`.
                let _ = handle.stop();
                Ok(())
            });
            handle.clone().stopped().await;
            Ok(())
        })
        .await
    }
}
