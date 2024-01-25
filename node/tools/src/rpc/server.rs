use std::net::SocketAddr;

use super::methods::health_check;
use jsonrpsee::server::{middleware::http::ProxyGetRequestLayer, RpcModule, Server};

pub async fn run_server(ip_address: SocketAddr) -> anyhow::Result<()> {
    // Custom tower service to handle the RPC requests
    let service_builder = tower::ServiceBuilder::new()
        // Proxy `GET /health` requests to internal `system_health` method.
        .layer(ProxyGetRequestLayer::new(
            "/health",
            health_check::method(),
        )?);

    let server = Server::builder()
        .set_http_middleware(service_builder)
        .build(ip_address)
        .await?;
    let mut module = RpcModule::new(());
    module.register_method(health_check::method(), |params, _| {
        health_check::callback(params)
    })?;

    let handle = server.start(module);

    // In this example we don't care about doing shutdown so let's it run forever.
    // You may use the `ServerHandle` to shut it down or manage it yourself.
    tokio::spawn(handle.stopped());

    Ok(())
}
