//! Deployer for the kubernetes cluster.
use std::net::SocketAddr;
use std::str::FromStr;
use std::{fs, path::PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use zksync_consensus_crypto::{Text, TextFmt};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_tools::{k8s, AppConfig, NodeAddr, NODES_PORT};

/// K8s namespace for consensus nodes.
const NAMESPACE: &str = "consensus";

/// Command line arguments.
#[derive(Debug, Parser)]
#[command(name = "deployer")]
struct DeployerCLI {
    /// Subcommand to run.
    #[command(subcommand)]
    command: DeployerCommands,
}

/// Subcommand arguments.
#[derive(Debug, Parser)]
struct SubCommandArgs {
    /// Number of total nodes to deploy.
    #[arg(long)]
    nodes: usize,
    /// Number of seed nodes to deploy.
    #[arg(long)]
    seed_nodes: Option<usize>,
}

/// Subcommands.
#[derive(Subcommand, Debug)]
enum DeployerCommands {
    /// Generate configs for the nodes.
    GenerateConfig(SubCommandArgs),
    /// Deploy the nodes.
    Deploy(SubCommandArgs),
}

/// Generates config for the nodes to run in the kubernetes cluster
/// Creates a directory for each node in the parent k8s_configs directory.
fn generate_config(nodes: usize) -> anyhow::Result<()> {
    assert!(nodes > 0, "at least 1 node has to be specified");

    // Generate the keys for all the replicas.
    let rng = &mut rand::thread_rng();

    let setup = validator::testonly::Setup::new(rng, nodes);
    let validator_keys = setup.keys.clone();

    // Each node will have `gossip_peers` outbound peers.
    let peers = 2;

    let node_keys: Vec<node::SecretKey> = (0..nodes).map(|_| node::SecretKey::generate()).collect();

    let default_config = AppConfig::default_for(setup.genesis.clone());

    let mut cfgs: Vec<_> = (0..nodes).map(|_| default_config.clone()).collect();

    // Construct a gossip network with optimal diameter.
    for (i, node) in node_keys.iter().enumerate() {
        for j in 0..peers {
            let next = (i * peers + j + 1) % nodes;
            cfgs[next].add_gossip_static_inbound(node.public());
        }
    }

    let manifest_path = std::env::var("CARGO_MANIFEST_DIR")?;
    let root = PathBuf::from(manifest_path).join("k8s_configs");
    let _ = fs::remove_dir_all(&root);
    for (i, cfg) in cfgs.into_iter().enumerate() {
        let node_config_dir = root.join(format!("consensus-node-{i:0>2}"));
        fs::create_dir_all(&node_config_dir)
            .with_context(|| format!("create_dir_all({:?})", node_config_dir))?;

        cfg.write_to_file(&node_config_dir)?;
        fs::write(
            node_config_dir.join("validator_key"),
            &TextFmt::encode(&validator_keys[i]),
        )
        .context("fs::write()")?;
        fs::write(
            node_config_dir.join("node_key"),
            &TextFmt::encode(&node_keys[i]),
        )
        .context("fs::write()")?;
    }

    Ok(())
}

/// Deploys the nodes to the kubernetes cluster.
async fn deploy(nodes: usize, seed_nodes: Option<usize>) -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::create_or_reuse_namespace(&client, NAMESPACE).await?;

    let seed_nodes = seed_nodes.unwrap_or(1);

    // deploy seed peer(s)
    for i in 0..seed_nodes {
        k8s::deploy_node(
            &client,
            i,
            true,
            vec![], // Seed peers don't have other peer information
            NAMESPACE,
        )
        .await?;
    }

    // obtain seed peer(s) IP(s)
    let peer_ips = k8s::get_seed_node_addrs(&client, seed_nodes, NAMESPACE).await?;

    let mut peers = vec![];

    for i in 0..seed_nodes {
        let node_id = &format!("consensus-node-{i:0>2}");
        let node_key = read_node_key_from_config(node_id)?;
        let address = peer_ips.get(node_id).context("IP address not found")?;
        peers.push(NodeAddr {
            key: node_key.public(),
            addr: SocketAddr::from_str(&format!("{address}:{NODES_PORT}"))?,
        });
    }

    // deploy the rest of nodes
    for i in seed_nodes..nodes {
        k8s::deploy_node(&client, i, false, peers.clone(), NAMESPACE).await?;
    }

    Ok(())
}

/// Obtain node key from config file.
fn read_node_key_from_config(node_id: &String) -> anyhow::Result<node::SecretKey> {
    let manifest_path = std::env::var("CARGO_MANIFEST_DIR")?;
    let root = PathBuf::from(manifest_path).join("k8s_configs");
    let node_key_path = root.join(node_id).join("node_key");
    let key = fs::read_to_string(node_key_path).context("failed reading file")?;
    Text::new(&key).decode().context("failed decoding key")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let DeployerCLI { command } = DeployerCLI::parse();

    match command {
        DeployerCommands::GenerateConfig(args) => generate_config(args.nodes),
        DeployerCommands::Deploy(args) => deploy(args.nodes, args.seed_nodes).await,
    }
}
