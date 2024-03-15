//! This is a simple test for the RPC server. It checks if the server is running and can respond to.
use anyhow::{ensure, Context};
use clap::{Parser, Subcommand};
use jsonrpsee::{core::client::ClientT, http_client::HttpClientBuilder, rpc_params, types::Params};
use std::{fs, io::Write, net::SocketAddr, path::PathBuf, str::FromStr};
use zksync_consensus_tools::{
    k8s,
    rpc::methods::{health_check::HealthCheck, RPCMethod},
};

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
    /// Generate configs for the nodes.
    GenerateConfig,
    /// Set up the test pod.
    StartPod,
    /// Deploy the nodes.
    Run,
}

/// Get the path of the node ips config file.
/// This way we can run the test from every directory and also inside kubernetes pod.
fn get_config_path() -> PathBuf {
    // This way we can run the test from every directory and also inside kubernetes pod.
    let manifest_path = std::env::var("CARGO_MANIFEST_DIR");
    if let Ok(manifest) = manifest_path {
        PathBuf::from(&format!("{}/config.txt", manifest))
    } else {
        PathBuf::from("config.txt")
    }
}

/// Generate a config file with the IPs of the consensus nodes in the kubernetes cluster.
pub async fn generate_config() -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    let pods_ip = k8s::get_consensus_nodes_address(&client)
        .await
        .context("Failed to get consensus pods address")?;
    ensure!(
        !pods_ip.is_empty(),
        "No consensus pods found in the k8s cluster"
    );
    let config_file_path = get_config_path();
    let mut config_file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(config_file_path)?;
    for addr in pods_ip {
        writeln!(config_file, "{addr}").context("Failed to write to config file")?;
    }
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
    let config_file_path = get_config_path();
    let nodes_socket = fs::read_to_string(config_file_path).unwrap();
    for socket in nodes_socket.lines() {
        let socket = SocketAddr::from_str(socket).unwrap();
        let url = format!("http://{}", socket);
        let rpc_client = HttpClientBuilder::default().build(url).unwrap();
        let params = Params::new(None);
        let response: serde_json::Value = rpc_client
            .request(HealthCheck::method(), rpc_params!())
            .await
            .unwrap();
        assert_eq!(response, HealthCheck::callback(params).unwrap());
    }
}

/// Main function for the test.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = TesterCLI::parse();
    tracing_subscriber::fmt::init();

    match args.command {
        TesterCommands::GenerateConfig => generate_config().await,
        TesterCommands::StartPod => start_tests_pod().await,
        TesterCommands::Run => {
            tracing::info!("Running sanity test");
            sanity_test().await;
            tracing::info!("Test Passed!");
            Ok(())
        }
    }
}
