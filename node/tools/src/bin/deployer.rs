//! Deployer for the kubernetes cluster.
use std::{fs, net::SocketAddr, path::PathBuf};

use anyhow::Context;
use clap::Parser;
use rand::Rng;
use zksync_consensus_bft::testonly;
use zksync_consensus_crypto::TextFmt;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_tools::AppConfig;

/// Ports for the nodes to listen on kubernetes pod.
const NODES_PORT: u16 = 3054;

/// Command line arguments.
#[derive(Debug, Parser)]
struct Args {
    /// Amount of nodes to deploy.
    #[arg(long)]
    nodes: usize,
}

/// Encodes a generated proto message to json for arbitrary ProtoFmt.
fn encode_json<T: zksync_protobuf::ProtoFmt>(x: &T) -> String {
    let mut s = serde_json::Serializer::pretty(vec![]);
    zksync_protobuf::serde::serialize(x, &mut s).unwrap();
    String::from_utf8(s.into_inner()).unwrap()
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let nodes = args.nodes;
    assert!(nodes > 0, "at least 1 node has to be specified");

    // Generate the keys for all the replicas.
    let rng = &mut rand::thread_rng();
    let validator_keys: Vec<validator::SecretKey> = (0..nodes).map(|_| rng.gen()).collect();
    let node_keys: Vec<node::SecretKey> = (0..nodes).map(|_| rng.gen()).collect();

    // Generate the genesis block.
    // TODO: generating genesis block shouldn't require knowing the private keys.
    let (genesis, validator_set) = testonly::make_genesis(
        &validator_keys,
        validator::Payload(vec![]),
        validator::BlockNumber(0),
    );

    // Each node will have `gossip_peers` outbound peers.
    let peers = 2;

    let mut cfgs: Vec<_> = (0..args.nodes)
        .map(|_| AppConfig {
            server_addr: SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), NODES_PORT),
            public_addr: SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), NODES_PORT),
            metrics_server_addr: None,

            validators: validator_set.clone(),
            genesis_block: genesis.clone(),

            gossip_dynamic_inbound_limit: 2,
            gossip_static_inbound: [].into(),
            gossip_static_outbound: [].into(),
        })
        .collect();

    // Construct a gossip network with optimal diameter.
    for (i, node_key) in node_keys.iter().enumerate() {
        for j in 0..peers {
            let next = (i * peers + j + 1) % args.nodes;
            cfgs[next].gossip_static_inbound.insert(node_key.public());
        }
    }

    let manifest_path = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = PathBuf::from(manifest_path).join("k8s_configs");
    let _ = fs::remove_dir_all(&root);
    for (i, cfg) in cfgs.into_iter().enumerate() {
        // Recreate the directory for the node's config.
        let node_config_dir = root.join(format!("node_{}", i));
        fs::create_dir_all(&node_config_dir)
            .with_context(|| format!("create_dir_all({:?})", node_config_dir))?;

        fs::write(node_config_dir.join("config.json"), encode_json(&cfg)).context("fs::write()")?;
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
