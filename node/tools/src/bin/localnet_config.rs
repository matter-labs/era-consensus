//! This tool constructs collection of node configs for running tests.
use anyhow::Context as _;
use clap::Parser;
use rand::{seq::SliceRandom as _, Rng};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, Permissions},
    net::{Ipv4Addr, SocketAddr},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};
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
    /// How many of the nodes from `input_addrs` should be validators.
    /// By default all nodes are validators.
    #[arg(long)]
    validator_count: Option<usize>,
    /// How many gossipnet peers should each node have.
    #[arg(long, default_value_t = 2)]
    peer_count: usize,
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
    anyhow::ensure!(!addrs.is_empty(), "at least 1 address has to be specified");
    let validator_count = args.validator_count.unwrap_or(addrs.len());
    anyhow::ensure!(validator_count <= addrs.len());

    // Generate the keys for all the replicas.
    let rng = &mut rand::thread_rng();

    let setup = validator::testonly::Setup::new(rng, validator_count);
    let validator_keys = setup.validator_keys.clone();

    // Each node will have `gossip_peers` outbound peers.
    let nodes = addrs.len();
    let peers = nodes.min(args.peer_count);

    let node_keys: Vec<node::SecretKey> = (0..nodes).map(|_| rng.gen()).collect();
    let mut cfgs: Vec<_> = (0..nodes)
        .map(|i| AppConfig {
            server_addr: SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), addrs[i].port()),
            public_addr: addrs[i].into(),
            rpc_addr: None,
            metrics_server_addr: args
                .metrics_server_port
                .map(|port| SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port)),
            genesis: setup.genesis.clone(),
            max_payload_size: 1000000,
            node_key: node_keys[i].clone(),
            validator_key: validator_keys.get(i).cloned(),
            gossip_dynamic_inbound_limit: peers,
            gossip_static_inbound: HashSet::default(),
            gossip_static_outbound: HashMap::default(),
            debug_page: None,
        })
        .collect();

    // Construct a gossip network with optimal diameter.
    {
        let mut cfgs: Vec<_> = cfgs.iter_mut().collect();
        cfgs.shuffle(rng);
        let mut next = 0;
        for i in 0..cfgs.len() {
            cfgs[i].gossip_static_outbound = (0..peers)
                .map(|_| {
                    next = (next + 1) % nodes;
                    (cfgs[next].node_key.public(), cfgs[next].public_addr.clone())
                })
                .collect();
        }
    }

    for cfg in cfgs {
        // Recreate the directory for the node's config.
        let root = args.output_dir.join(&cfg.public_addr.0);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).with_context(|| format!("create_dir_all({:?})", root))?;
        fs::set_permissions(root.clone(), Permissions::from_mode(0o700))
            .context("fs::set_permissions()")?;

        let config_path = root.join("config.json");
        fs::write(&config_path, encode_json(&Serde(cfg))).context("fs::write()")?;
        fs::set_permissions(&config_path, Permissions::from_mode(0o600))
            .context("fs::set_permissions()")?;
    }
    Ok(())
}
