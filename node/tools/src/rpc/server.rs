use crate::AppConfig;

use super::methods::{
    config::ConfigInfo, health_check::HealthCheck, last_view::LastView, peers::PeersInfo, RPCMethod,
};
use jsonrpsee::server::{middleware::http::ProxyGetRequestLayer, RpcModule, Server};
use std::{net::SocketAddr, sync::Arc};
use zksync_concurrency::{ctx, scope};
use zksync_consensus_storage::BlockStore;

/// RPC server.
pub struct RPCServer {
    /// IP address to bind to.
    ip_address: SocketAddr,
    /// AppConfig
    config: AppConfig,
    node_storage: Arc<BlockStore>,
}

impl RPCServer {
    pub fn new(ip_address: SocketAddr, config: AppConfig, node_storage: Arc<BlockStore>) -> Self {
        Self {
            ip_address,
            config,
            node_storage,
        }
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
            )?)
            .layer(ProxyGetRequestLayer::new(
                LastView::path(),
                LastView::method(),
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

        let node_storage = self.node_storage.clone();
        module.register_method(LastView::method(), move |_params, _| {
            LastView::info(node_storage.clone())
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
