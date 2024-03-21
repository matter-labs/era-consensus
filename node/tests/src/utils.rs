//! This module contains utility functions for the test binary.
use anyhow::Context;
use jsonrpsee::{
    core::client::ClientT,
    http_client::{HttpClient, HttpClientBuilder},
    rpc_params,
};
use zksync_consensus_tools::{
    k8s::{self, PodId},
    rpc::methods::{health_check, last_commited_block},
};

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

pub(crate) async fn add_chaos_delay_for_target_pods(
    target_pods: Vec<PodId>,
    delay: u8,
) -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    for pod_id in target_pods {
        k8s::chaos::add_chaos_delay_for_pod(&client, pod_id, delay).await?;
    }
    Ok(())
}

pub(crate) async fn get_last_commited_block(rpc_client: HttpClient) -> anyhow::Result<u64> {
    let response: serde_json::Value = rpc_client
        .request(last_commited_block::method(), rpc_params!())
        .await
        .context("Failed to get last commited block")?;
    let last_commited_block: u64 =
        serde_json::from_value(response.get("last_commited_block").unwrap().to_owned())
            .context("Failed to parse last commited block")?;
    Ok(last_commited_block)
}

pub(crate) async fn check_health_of_node(rpc_client: HttpClient) -> anyhow::Result<bool> {
    let response: serde_json::Value = rpc_client
        .request(health_check::method(), rpc_params!())
        .await
        .context("Failed to get last commited block")?;
    let health_check: bool = serde_json::from_value(response.get("health").unwrap().to_owned())
        .context("Failed to parse health check")?;
    Ok(health_check)
}

pub(crate) async fn get_consensus_node_rpc_client(node_id: &PodId) -> anyhow::Result<HttpClient> {
    let client = k8s::get_client().await?;
    let socket = k8s::get_node_rpc_address_with_id(&client, &node_id)
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
    k8s::chaos::create_or_reuse_network_chaos_role(&client).await?;
    Ok(())
}

/// Deploy a service to expose the tester RPC server.
pub(crate) async fn deploy_rpc_service() -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::expose_tester_rpc(&client).await?;
    Ok(())
}
