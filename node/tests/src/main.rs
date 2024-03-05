//! This is a simple test for the RPC server. It checks if the server is running and can respond to.
use std::{fs, io::Write, path::PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use jsonrpsee::{core::client::ClientT, http_client::HttpClientBuilder, rpc_params};
use zksync_consensus_tools::{k8s, rpc::methods::health_check};

/// Command line arguments.
#[derive(Debug, Parser)]
#[command(name = "tester")]
struct TesterCLI {
    /// Subcommand to run.
    #[command(subcommand)]
    command: TesterCommands,
}

/// Subcommands.
#[derive(Subcommand, Debug)]
enum TesterCommands {
    /// Set up the test pod.
    StartPod,
    /// Deploy the nodes.
    Run,
}

pub(crate) async fn deploy_role() -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::create_or_reuse_namespace(&client, k8s::DEFAULT_NAMESPACE).await?;
    k8s::create_or_reuse_pod_reader_role(&client).await?;
    k8s::create_or_reuse_network_chaos_role(&client).await?;
    Ok(())
}

/// Start the tests pod in the kubernetes cluster.
pub async fn start_tests_pod() -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::create_tests_deployment(&client)
        .await
        .context("Failed to create tests pod")?;
    Ok(())
}

/// Sanity test for the RPC server.
/// We use unwraps here because this function is intended to be used like a test.
pub async fn sanity_test() {
    let client = k8s::get_client().await.unwrap();
    let nodes_socket = k8s::get_consensus_nodes_address(&client).await.unwrap();
    for socket in nodes_socket {
        let url: String = format!("http://{}", socket);
        let rpc_client = HttpClientBuilder::default().build(url).unwrap();
        let response: serde_json::Value = rpc_client
            .request(health_check::method(), rpc_params!())
            .await
            .unwrap();
        assert_eq!(response, health_check::callback().unwrap());
    }
}

/// Sanity test for the RPC server.
/// We use unwraps here because this function is intended to be used like a test.
pub async fn delay_test() {
    let client = k8s::get_client().await.unwrap();
    k8s::add_chaos_delay_for_node(&client, "consensus-node-01")
        .await
        .unwrap();
}

/// Main function for the test.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = TesterCLI::parse();
    tracing_subscriber::fmt::init();

    match args.command {
        TesterCommands::StartPod => {
            deploy_role().await?;
            start_tests_pod().await
        }
        TesterCommands::Run => {
            tracing::info!("Running sanity test");
            sanity_test().await;
            tracing::info!("Test Passed!");
            tracing::info!("Running Delay tests");
            delay_test().await;
            tracing::info!("Test Passed!");
            Ok(())
        }
    }
}
