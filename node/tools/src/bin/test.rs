//! asdfasdf

use clap::{command, Args, Parser};
use std::path::PathBuf;

/// Command-line application launching a node executor.
#[derive(Debug, Parser)]
/// asdfasdf
struct Cli {
    /// Node configuration.

    /// Command line config
    /// Full json config
    #[arg(long, requires="node_key", conflicts_with_all=["config_file", "validator_key_file", "node_key_file"])]
    config: Option<String>,
    /// Plain node key
    #[arg(long, requires="config", conflicts_with_all=["config_file", "validator_key_file", "node_key_file"])]
    node_key: Option<String>,
    /// Plain validator key
    #[arg(long, conflicts_with_all=["config_file", "validator_key_file", "node_key_file"])]
    validator_key: Option<String>,

    /// Path to a validator key file. If set to an empty string, validator key will not be read
    /// (i.e., a node will be initialized as a non-validator node).
    #[arg(long, default_value = "./validator_key")]
    validator_key_file: PathBuf,
    /// Path to a JSON file with node configuration.
    #[arg(long, default_value = "./config.json")]
    config_file: PathBuf,
    /// Path to a node key file.
    #[arg(long, default_value = "./node_key")]
    node_key_file: PathBuf,

    /// Path to the rocksdb database of the node.
    #[arg(long, default_value = "./database")]
    database: PathBuf,
    /// Port for the RPC server.
    #[arg(long)]
    rpc_port: Option<u16>,
}

/// adasdsa
#[tokio::main]
/// adasdsa
async fn main() -> anyhow::Result<()> {
    let args: Cli = Cli::parse();
    dbg!(args);
    Ok(())
}
