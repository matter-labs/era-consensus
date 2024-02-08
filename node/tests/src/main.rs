//! This is a simple test for the RPC server. It checks if the server is running and can respond to.
use jsonrpsee::{core::client::ClientT, http_client::HttpClientBuilder, rpc_params, types::Params};
use zksync_consensus_tools::rpc::methods::{health_check::HealthCheck, RPCMethod};

/// Sanity test for the RPC server.
pub async fn sanity_test() {
    let url = "http://127.0.0.1:3154";
    let rpc_client = HttpClientBuilder::default().build(url).unwrap();
    let params = Params::new(None);
    let response: serde_json::Value = rpc_client
        .request(HealthCheck::method(), rpc_params!())
        .await
        .unwrap();
    assert_eq!(response, HealthCheck::callback(params).unwrap());
}

/// Main function for the test.
#[tokio::main]
async fn main() {
    sanity_test().await;
    println!("IT WORKS!");
}
