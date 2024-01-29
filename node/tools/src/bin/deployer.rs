//! Deployer for the kubernetes cluster.
use std::{fs, path::PathBuf};

use anyhow::Context;
use clap::Parser;
use rand::Rng;
use zksync_consensus_crypto::TextFmt;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_tools::AppConfig;

/// Command line arguments.
#[derive(Debug, Parser)]
struct Args {
    /// Amount of nodes to deploy.
    #[arg(long)]
    nodes: usize,
}

fn generate_config() -> anyhow::Result<Vec<validator::SecretKey>> {
    let args = Args::parse();
    let nodes = args.nodes;
    assert!(nodes > 0, "at least 1 node has to be specified");

    // Generate the keys for all the replicas.
    let rng = &mut rand::thread_rng();
    let node_keys: Vec<node::SecretKey> = (0..nodes).map(|_| rng.gen()).collect();

    // Each node will have `gossip_peers` outbound peers.
    let peers = 2;

    let (default_config, validator_keys) = AppConfig::default_for(nodes as u64);
    let mut cfgs: Vec<_> = (0..args.nodes).map(|_| default_config.clone()).collect();

    // Construct a gossip network with optimal diameter.
    for (i, node_key) in node_keys.iter().enumerate() {
        for j in 0..peers {
            let next = (i * peers + j + 1) % args.nodes;
            cfgs[next].add_gossip_static_inbound(node_key.public());
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
