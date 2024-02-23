use crate::AppConfig;

use super::methods::{config, health_check, last_view, peers};
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
    /// Node storage.
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
                health_check::path(),
                health_check::method(),
            )?)
            .layer(ProxyGetRequestLayer::new(peers::path(), peers::method())?)
            .layer(ProxyGetRequestLayer::new(config::path(), config::method())?)
            .layer(ProxyGetRequestLayer::new(
                last_view::path(),
                last_view::method(),
            )?);

        let mut module = RpcModule::new(());
        module.register_method(health_check::method(), |_params, _| {
            health_check::callback()
        })?;
        module.register_method(peers::method(), |_params, _| peers::callback())?;

        // TODO find a better way to implement this as I had to clone the clone and move it to pass the borrow checker
        let config = self.config.clone();
        module.register_method(config::method(), move |_params, _| {
            config::callback(config.clone())
        })?;

        let node_storage = self.node_storage.clone();
        module.register_method(last_view::method(), move |_params, _| {
            last_view::callback(node_storage.clone())
        })?;

        let server = Server::builder()
            .set_http_middleware(service_builder)
            .build(self.ip_address)
            .await?;

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
