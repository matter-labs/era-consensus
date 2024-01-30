use std::net::SocketAddr;

use super::methods::{health_check::HealthCheck, RPCMethod};
use jsonrpsee::server::{middleware::http::ProxyGetRequestLayer, RpcModule, Server};
use zksync_concurrency::ctx::Ctx;

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
    pub async fn run(&self, ctx: &Ctx) -> anyhow::Result<()> {
        // Custom tower service to handle the RPC requests
        let service_builder = tower::ServiceBuilder::new()
            // Proxy `GET /health` requests to internal `system_health` method.
            .layer(ProxyGetRequestLayer::new(
                HealthCheck::path(),
                HealthCheck::method(),
            )?);

        let server = Server::builder()
            .set_http_middleware(service_builder)
            .build(self.ip_address)
            .await?;

        let mut module = RpcModule::new(());
        module.register_method(HealthCheck::method(), |params, _| {
            HealthCheck::callback(params)
        })?;

        let handle = server.start(module);

        ctx.wait(handle.stopped()).await?;
        Ok(())
    }
}
