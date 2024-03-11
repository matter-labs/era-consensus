//! This module contains utility functions for the test binary.
use anyhow::Context;
use zksync_consensus_tools::k8s;

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
    k8s::create_or_reuse_network_chaos_role(&client).await?;
    Ok(())
}

/// Deploy a service to expose the tester RPC server.
pub(crate) async fn deploy_rpc_service() -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::expose_tester_rpc(&client).await?;
    Ok(())
}
