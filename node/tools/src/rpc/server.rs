use super::methods::{config::ConfigInfo, health_check::HealthCheck, peers::PeersInfo, RPCMethod};
use crate::AppConfig;
use jsonrpsee::server::{middleware::http::ProxyGetRequestLayer, RpcModule, Server};
use std::net::SocketAddr;
use zksync_concurrency::{ctx, scope};

/// RPC server.
pub struct RPCServer {
    /// IP address to bind to.
    ip_address: SocketAddr,
    /// AppConfig
    config: AppConfig,
}

impl RPCServer {
    pub fn new(ip_address: SocketAddr, config: AppConfig) -> Self {
        Self { ip_address, config }
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
            )?)
            .layer(ProxyGetRequestLayer::new(
                ConfigInfo::path(),
                ConfigInfo::method(),
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

        // TODO find a better way to implement this as I had to clone the clone and move it to pass the borrow checker
        let config = self.config.clone();
        module.register_method(ConfigInfo::method(), move |_params, _| {
            ConfigInfo::info(config.clone())
        })?;

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
