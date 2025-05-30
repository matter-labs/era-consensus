use std::{net::SocketAddr, sync::Arc};

use jsonrpsee::server::{middleware::http::ProxyGetRequestLayer, RpcModule, Server};
use zksync_concurrency::{ctx, scope};
use zksync_consensus_engine::EngineManager;

use super::methods::{health_check, last_committed_block, last_view};

/// RPC server.
pub struct RPCServer {
    /// IP address to bind to.
    ip_address: SocketAddr,
    /// Node storage.
    node_engine: Arc<EngineManager>,
}

impl RPCServer {
    pub fn new(ip_address: SocketAddr, node_engine: Arc<EngineManager>) -> Self {
        Self {
            ip_address,
            node_engine,
        }
    }

    /// Runs the RPC server.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        // Custom tower service to handle the RPC requests
        let service_builder = tower::ServiceBuilder::new()
            // Proxy `GET /<path>` requests to internal methods.
            .layer(ProxyGetRequestLayer::new(
                health_check::path(),
                health_check::method(),
            )?)
            .layer(ProxyGetRequestLayer::new(
                last_view::path(),
                last_view::method(),
            )?)
            .layer(ProxyGetRequestLayer::new(
                last_committed_block::path(),
                last_committed_block::method(),
            )?);

        let mut module = RpcModule::new(());
        module.register_method(health_check::method(), |_params, _, _| {
            health_check::callback()
        })?;

        let node_engine = self.node_engine.clone();
        module.register_method(last_view::method(), move |_params, _, _| {
            last_view::callback(node_engine.clone())
        })?;

        let node_engine = self.node_engine.clone();
        module.register_method(last_committed_block::method(), move |_params, _, _| {
            last_committed_block::callback(node_engine.clone())
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
