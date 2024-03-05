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

pub(crate) async fn deploy_role() -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::create_or_reuse_namespace(&client, k8s::DEFAULT_NAMESPACE).await?;
    k8s::create_or_reuse_pod_reader_role(&client).await?;
    k8s::create_or_reuse_network_chaos_role(&client).await?;
    Ok(())
}
