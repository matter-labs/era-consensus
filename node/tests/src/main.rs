//! This is a simple test for the RPC server. It checks if the server is running and can respond to.
use std::{fs, io::Write};

use clap::{Parser, Subcommand};
use jsonrpsee::{core::client::ClientT, http_client::HttpClientBuilder, rpc_params, types::Params};
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
    /// Deploy the nodes.
    Run,
}

fn get_config_path() -> String {
    // This way we can run the test from every directory and also inside kubernetes pod.
    let manifest_path = std::env::var("CARGO_MANIFEST_DIR");
    if let Ok(manifest) = manifest_path {
        format!("{}/config.txt", manifest)
    } else {
        "config.txt".to_owned()
    }
}

/// Generate a config file with the IPs of the consensus nodes in the kubernetes cluster.
pub async fn generate_config() {
    let client = k8s::get_client().await.unwrap();
    let pods_ip = k8s::get_consensus_node_ips(&client).await.unwrap();
    let config_file_path: String = get_config_path();
    for ip in pods_ip {
        let mut config_file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&config_file_path)
            .unwrap();
        config_file.write_all(ip.as_bytes()).unwrap();
    }
}

/// Sanity test for the RPC server.
pub async fn sanity_test() {
    let config_file_path = get_config_path();
    let node_ips = fs::read_to_string(config_file_path).unwrap();
    for ip in node_ips.lines() {
        let url: String = format!("http://{}:3154", ip);
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
async fn main() {
    let args = TesterCLI::parse();
    tracing::trace!(?args, "Starting node");
    match args.command {
        TesterCommands::GenerateConfig => {
            generate_config().await;
            tracing::info!("Config succesfully generated")
        }
        TesterCommands::Run => {
            sanity_test().await;
            tracing::info!("Test passed!")
        }
    }
}
