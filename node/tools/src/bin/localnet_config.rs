//! This tool constructs collection of node configs for running tests.
use anyhow::Context as _;
use clap::Parser;
use rand::Rng;
use std::{
    collections::{HashMap, HashSet},
    fs,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
};
use zksync_consensus_crypto::TextFmt;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_tools::{encode_json, AppConfig};
use zksync_protobuf::serde::Serde;

/// Command line arguments.
#[derive(Debug, Parser)]
struct Args {
    /// Path to a file with newline separated IP:port addrs of the nodes to configure.
    /// Binary will generate a config for each IP in this file.
    #[arg(long)]
    input_addrs: PathBuf,
    /// TCP port to serve metrics for scraping.
    #[arg(long)]
    metrics_server_port: Option<u16>,
    /// Path to a directory in which the configs should be created.
    /// Configs for <ip:port>, will be in directory <output_dir>/<ip:port>/
    #[arg(long)]
    output_dir: PathBuf,
    /// Block payload size in bytes.
    #[arg(long, default_value_t = 1000000)]
    payload_size: usize,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let addrs_raw = fs::read_to_string(&args.input_addrs)
        .with_context(|| args.input_addrs.to_str().unwrap().to_owned())?;
    let mut addrs = vec![];
    for a in addrs_raw.split_whitespace() {
        addrs.push(
            a.parse::<SocketAddr>()
                .with_context(|| format!("parse('{}')", a))?,
        );
    }
    assert!(!addrs.is_empty(), "at least 1 address has to be specified");

    // Generate the keys for all the replicas.
    let rng = &mut rand::thread_rng();

    let setup = validator::testonly::Setup::new(rng, addrs.len());
    let validator_keys = setup.keys.clone();

    // Each node will have `gossip_peers` outbound peers.
    let nodes = addrs.len();
    let peers = 2;

    let node_keys: Vec<node::SecretKey> = (0..nodes).map(|_| rng.gen()).collect();
    let mut cfgs: Vec<_> = (0..nodes)
        .map(|i| AppConfig {
            server_addr: SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), addrs[i].port()),
            public_addr: addrs[i].into(),
            debug_addr: None,
            metrics_server_addr: args
                .metrics_server_port
                .map(|port| SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port)),
            genesis: setup.genesis.clone(),
            max_payload_size: 1000000,
            gossip_dynamic_inbound_limit: 0,
            gossip_static_inbound: HashSet::default(),
            gossip_static_outbound: HashMap::default(),
        })
        .collect();

    // Construct a gossip network with optimal diameter.
    for i in 0..nodes {
        for j in 0..peers {
            let next = (i * peers + j + 1) % nodes;
            cfgs[i]
                .gossip_static_outbound
                .insert(node_keys[next].public(), addrs[next].into());
            cfgs[next]
                .gossip_static_inbound
                .insert(node_keys[i].public());
        }
    }

    for (i, cfg) in cfgs.into_iter().enumerate() {
        // Recreate the directory for the node's config.
        let root = args.output_dir.join(&cfg.public_addr.0);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).with_context(|| format!("create_dir_all({:?})", root))?;
        fs::write(root.join("config.json"), encode_json(&Serde(cfg))).context("fs::write()")?;
        fs::write(
            root.join("validator_key"),
            &TextFmt::encode(&validator_keys[i]),
        )
        .context("fs::write()")?;
        fs::write(root.join("node_key"), &TextFmt::encode(&node_keys[i])).context("fs::write()")?;
    }
    Ok(())
}
