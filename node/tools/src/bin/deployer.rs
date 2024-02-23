//! Deployer for the kubernetes cluster.
use std::collections::HashMap;
use std::{fs, path::PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use zksync_consensus_crypto::{Text, TextFmt};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_tools::{k8s, AppConfig};

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
async fn deploy(nodes_amount: usize, seed_nodes_amount: Option<usize>) -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    k8s::create_or_reuse_namespace(&client, NAMESPACE).await?;
    let seed_nodes_amount = seed_nodes_amount.unwrap_or(1);

    let seed_nodes = &mut HashMap::new();
    let mut non_seed_nodes = HashMap::new();

    // Split the nodes in different hash maps as they will be deployed at different stages
    let mut consensus_nodes = from_configs(nodes_amount)?;
    for (index, node) in consensus_nodes.iter_mut().enumerate() {
        if index < seed_nodes_amount {
            node.is_seed = true;
            seed_nodes.insert(node.id.to_owned(), node);
        } else {
            non_seed_nodes.insert(node.id.to_owned(), node);
        }
    }

    // Deploy seed peer(s)
    for node in seed_nodes.values() {
        k8s::deploy_node(&client, node, NAMESPACE).await?;
    }

    // Fetch and complete node addrs into seed nodes
    k8s::find_node_addrs(&client, seed_nodes, NAMESPACE).await?;

    // Build a vector of seed peers NodeAddrs to provide as gossip_static_outbound to the rest of the nodes
    let peers: Vec<_> = seed_nodes
        .values()
        .map(|n| {
            n.node_addr
                .as_ref()
                .expect("Seed node address not defined")
                .clone()
        })
        .collect();

    // Deploy the rest of the nodes
    for node in non_seed_nodes.values_mut() {
        node.gossip_static_outbound = peers.clone();
        k8s::deploy_node(&client, node, NAMESPACE).await?;
    }

    Ok(())
}

/// Build ConsensusNodes representation list from configurations
// TODO once we can provide config via cli args, this will be replaced
//      using in-memory config structs
fn from_configs(nodes: usize) -> anyhow::Result<Vec<k8s::ConsensusNode>> {
    let manifest_path = std::env::var("CARGO_MANIFEST_DIR")?;
    let root = PathBuf::from(manifest_path).join("k8s_configs");
    let mut consensus_nodes = vec![];

    for i in 0..nodes {
        let node_id = format!("consensus-node-{i:0>2}");
        let node_key_path = root.join(&node_id).join("node_key");
        let key_string = fs::read_to_string(node_key_path).context("failed reading file")?;
        let key = Text::new(&key_string)
            .decode()
            .context("failed decoding key")?;
        consensus_nodes.push(k8s::ConsensusNode {
            id: node_id,
            key,
            node_addr: None,
            is_seed: false,
            gossip_static_outbound: vec![],
        });
    }
    Ok(consensus_nodes)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let DeployerCLI { command } = DeployerCLI::parse();

    match command {
        DeployerCommands::GenerateConfig(args) => generate_config(args.nodes),
        DeployerCommands::Deploy(args) => deploy(args.nodes, args.seed_nodes).await,
    }
}
