//! This tool constructs collection of node configs for running tests.
use anyhow::Context as _;
use clap::Parser;
use rand::Rng;
use std::{fs, net::SocketAddr, path::PathBuf};
use zksync_consensus_crypto::TextFmt;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_tools::AppConfig;

/// Encodes a generated proto message to json for arbitrary ProtoFmt.
fn encode_json<T: zksync_protobuf::ProtoFmt>(x: &T) -> String {
    let mut s = serde_json::Serializer::pretty(vec![]);
    zksync_protobuf::serde::serialize(x, &mut s).unwrap();
    String::from_utf8(s.into_inner()).unwrap()
}

/// Replaces IP of the address with UNSPECIFIED (aka INADDR_ANY) of the corresponding IP type.
/// Opening a listener socket with an UNSPECIFIED IP, means that the new connections
/// on any network interface of the VM will be accepted.
fn with_unspecified_ip(addr: SocketAddr) -> SocketAddr {
    let unspecified_ip = match addr {
        SocketAddr::V4(_) => std::net::Ipv4Addr::UNSPECIFIED.into(),
        SocketAddr::V6(_) => std::net::Ipv6Addr::UNSPECIFIED.into(),
    };
    SocketAddr::new(unspecified_ip, addr.port())
}

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
    let metrics_server_addr = args
        .metrics_server_port
        .map(|port| SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), port));

    // Generate the keys for all the replicas.
    let rng = &mut rand::thread_rng();

    let mut genesis = validator::GenesisSetup::empty(rng, addrs.len());
    genesis
        .next_block()
        .payload(validator::Payload(vec![]))
        .push();
    let validator_keys = genesis.keys.clone();
    let node_keys: Vec<node::SecretKey> = (0..addrs.len()).map(|_| rng.gen()).collect();

    // Each node will have `gossip_peers` outbound peers.
    let nodes = addrs.len();
    let peers = 2;

    let mut cfgs: Vec<_> = (0..nodes)
        .map(|i| AppConfig {
            server_addr: with_unspecified_ip(addrs[i]),
            public_addr: addrs[i],
            metrics_server_addr,

            validators: genesis.validator_set(),
            genesis_block: genesis.blocks[0].clone(),
            max_payload_size: args.payload_size,

            gossip_dynamic_inbound_limit: 0,
            gossip_static_inbound: [].into(),
            gossip_static_outbound: [].into(),
        })
        .collect();

    // Construct a gossip network with optimal diameter.
    for i in 0..nodes {
        for j in 0..peers {
            let next = (i * peers + j + 1) % nodes;
            cfgs[i]
                .gossip_static_outbound
                .insert(node_keys[next].public(), addrs[next]);
            cfgs[next]
                .gossip_static_inbound
                .insert(node_keys[i].public());
        }
    }

    for (i, cfg) in cfgs.into_iter().enumerate() {
        // Recreate the directory for the node's config.
        let root = args.output_dir.join(addrs[i].to_string());
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).with_context(|| format!("create_dir_all({:?})", root))?;

        fs::write(root.join("config.json"), encode_json(&cfg)).context("fs::write()")?;
        fs::write(
            root.join("validator_key"),
            &TextFmt::encode(&validator_keys[i]),
        )
        .context("fs::write()")?;
        fs::write(root.join("node_key"), &TextFmt::encode(&node_keys[i])).context("fs::write()")?;
    }
    Ok(())
}
