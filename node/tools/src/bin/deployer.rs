//! Deployer for the kubernetes cluster.
use clap::Parser;
use std::collections::HashMap;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_tools::k8s::ConsensusNode;
use zksync_consensus_tools::{k8s, AppConfig};

/// K8s namespace for consensus nodes.
const NAMESPACE: &str = "consensus";

/// Command line arguments.
#[derive(Debug, Parser)]
#[command(name = "deployer")]
struct DeployerCLI {
    /// Number of total nodes to deploy.
    #[arg(long)]
    nodes: usize,
    /// Number of seed nodes to deploy.
    #[arg(long)]
    seed_nodes: Option<usize>,
}

/// Generates the configuration for all the nodes to run in the kubernetes cluster
/// and creates a ConsensusNode for each to track their progress
fn generate_consensus_nodes(nodes: usize, seed_nodes_amount: Option<usize>) -> Vec<ConsensusNode> {
    assert!(nodes > 0, "at least 1 node has to be specified");
    let seed_nodes_amount = seed_nodes_amount.unwrap_or(1);

    // Generate the keys for all the replicas.
    let rng = &mut rand::thread_rng();

    let setup = validator::testonly::Setup::new(rng, nodes);
    let validator_keys = setup.keys.clone();

    // Each node will have `gossip_peers` outbound peers.
    let peers = 2;

    let node_keys: Vec<node::SecretKey> = (0..nodes).map(|_| node::SecretKey::generate()).collect();

    let default_config = AppConfig::default_for(setup.genesis.clone());

    let mut cfgs: Vec<ConsensusNode> = (0..nodes)
        .map(|i| ConsensusNode {
            id: format!("consensus-node-{i:0>2}"),
            config: default_config.clone(),
            key: node_keys[i].clone(),
            validator_key: Some(validator_keys[i].clone()),
            node_addr: None, //It's not assigned yet
            is_seed: i < seed_nodes_amount,
        })
        .collect();

    // Construct a gossip network with optimal diameter.
    for (i, node) in node_keys.iter().enumerate() {
        for j in 0..peers {
            let next = (i * peers + j + 1) % nodes;
            cfgs[next].config.add_gossip_static_inbound(node.public());
        }
    }

    cfgs
}

/// Deploys the nodes to the kubernetes cluster.
async fn deploy(nodes_amount: usize, seed_nodes_amount: Option<usize>) -> anyhow::Result<()> {
    let mut consensus_nodes = generate_consensus_nodes(nodes_amount, seed_nodes_amount);
    let client = k8s::get_client().await?;
    k8s::create_or_reuse_namespace(&client, NAMESPACE).await?;

    let seed_nodes = &mut HashMap::new();
    let mut non_seed_nodes = HashMap::new();

    // Split the nodes in different hash maps as they will be deployed at different stages
    for node in consensus_nodes.iter_mut() {
        if node.is_seed {
            seed_nodes.insert(node.id.to_owned(), node);
        } else {
            non_seed_nodes.insert(node.id.to_owned(), node);
        }
    }

    // Deploy seed peer(s)
    for node in seed_nodes.values_mut() {
        node.deploy(&client, NAMESPACE).await?;
    }

    // Fetch and complete node addrs into seed nodes
    for node in seed_nodes.values_mut() {
        node.fetch_and_assign_pod_ip(&client, NAMESPACE).await?;
    }

    // Build a vector of (PublicKey, SocketAddr) to provide as gossip_static_outbound
    // to the rest of the nodes
    let peers: Vec<_> = seed_nodes
        .values()
        .map(|n| {
            let node_addr = n
                .node_addr
                .as_ref()
                .expect("Seed node address not defined")
                .clone();
            (node_addr.key, node_addr.addr)
        })
        .collect();

    // Deploy the rest of the nodes
    for node in non_seed_nodes.values_mut() {
        node.config.gossip_static_outbound.extend(peers.clone());
        node.deploy(&client, NAMESPACE).await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = DeployerCLI::parse();
    deploy(args.nodes, args.seed_nodes).await
}
