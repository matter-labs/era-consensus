//! This module contains utility functions for the test binary.
use anyhow::Context;
use jsonrpsee::{
    core::client::ClientT,
    http_client::{HttpClient, HttpClientBuilder},
    rpc_params,
};
use zksync_consensus_tools::k8s::chaos_mesh;
use zksync_consensus_tools::{
    k8s::{self, PodId},
    rpc::methods::{health_check, last_committed_block},
};

/// Get the RPC clients for all the consensus nodes.
pub(crate) async fn get_consensus_nodes_rpc_client() -> anyhow::Result<Vec<HttpClient>> {
    let client = k8s::get_client().await?;
    let nodes_socket = k8s::get_consensus_nodes_rpc_address(&client).await?;
    let mut rpc_clients = Vec::new();
    for socket in nodes_socket {
        let url: String = format!("http://{}", socket);
        let rpc_client = HttpClientBuilder::default().build(url).unwrap();
        rpc_clients.push(rpc_client);
    }
    Ok(rpc_clients)
}

/// Add chaos delay for the target pods.
pub(crate) async fn add_chaos_delay_for_target_pods(target_pods: Vec<PodId>) -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    for pod_id in target_pods {
        chaos_mesh::ops::add_chaos_delay_for_pod(&client, pod_id).await?;
    }
    Ok(())
}

/// Get the last committed block using the rpc_client of the consensus node.
pub(crate) async fn get_last_committed_block(rpc_client: HttpClient) -> anyhow::Result<u64> {
    let response: serde_json::Value = rpc_client
        .request(last_committed_block::method(), rpc_params!())
        .await
        .context("Failed to get last committed block")?;
    let last_committed_block: u64 =
        serde_json::from_value(response.get("last_committed_block").unwrap().to_owned())
            .context("Failed to parse last committed block")?;
    Ok(last_committed_block)
}

/// Check the health of the consensus node using its rpc_client.
pub(crate) async fn check_health_of_node(rpc_client: HttpClient) -> anyhow::Result<bool> {
    let response: serde_json::Value = rpc_client
        .request(health_check::method(), rpc_params!())
        .await
        .context("Failed to get last committed block")?;
    let health_check: bool = serde_json::from_value(response.get("health").unwrap().to_owned())
        .context("Failed to parse health check")?;
    Ok(health_check)
}

/// Get the RPC client for the consensus node with the given pod ID.
pub(crate) async fn get_consensus_node_rpc_client(node_id: &PodId) -> anyhow::Result<HttpClient> {
    let client = k8s::get_client().await?;
    let socket = k8s::get_node_rpc_address_with_id(&client, node_id)
        .await
        .context("Failed to get node rpc address")?;
    let url: String = format!("http://{}", socket);
    let rpc_client = HttpClientBuilder::default().build(url).unwrap();
    Ok(rpc_client)
}

/// Start the tests pod in the kubernetes cluster.
pub(crate) async fn start_tests_pod() -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::create_tests_deployment(&client)
        .await
        .context("Failed to create tests pod")?;
    Ok(())
}

/// Deploy the necessary roles for the test pod.
/// This includes the role for listing the consensus pods and the role to deploy network chaos.
pub(crate) async fn deploy_role() -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::create_or_reuse_namespace(&client, k8s::DEFAULT_NAMESPACE).await?;
    k8s::create_or_reuse_pod_reader_role(&client).await?;
    chaos_mesh::ops::create_or_reuse_network_chaos_role(&client).await?;
    Ok(())
}

/// Deploy a service to expose the tester RPC server.
pub(crate) async fn deploy_rpc_service() -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::expose_tester_rpc(&client).await?;
    Ok(())
}
