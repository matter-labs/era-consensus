//! This tool constructs collection of node configs for running tests.
use anyhow::Context as _;
use clap::Parser;
use rand::Rng;
use std::{fs, net::SocketAddr, path::PathBuf};
use zksync_consensus_bft::testonly;
use zksync_consensus_crypto::TextFmt;
use zksync_consensus_executor::{ConsensusConfig, ExecutorConfig, GossipConfig};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_tools::NodeConfig;

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
    let validator_keys: Vec<validator::SecretKey> = (0..addrs.len()).map(|_| rng.gen()).collect();
    let node_keys: Vec<node::SecretKey> = (0..addrs.len()).map(|_| rng.gen()).collect();

    // Generate the genesis block.
    // TODO: generating genesis block shouldn't require knowing the private keys.
    let (genesis, validator_set) =
        testonly::make_genesis(&validator_keys, validator::Payload(vec![]));

    // Each node will have `gossip_peers` outbound peers.
    let nodes = addrs.len();
    let peers = 2;

    let mut gossip_cfgs: Vec<_> = node_keys
        .iter()
        .map(|k| GossipConfig {
            key: k.public(),
            dynamic_inbound_limit: 0,
            static_inbound: [].into(),
            static_outbound: [].into(),
        })
        .collect();

    // Construct a gossip network with optimal diameter.
    for i in 0..nodes {
        for j in 0..peers {
            let next = (i * peers + j + 1) % nodes;
            gossip_cfgs[i]
                .static_outbound
                .insert(node_keys[next].public(), addrs[next]);
            gossip_cfgs[next]
                .static_inbound
                .insert(node_keys[i].public());
        }
    }

    for (i, gossip) in gossip_cfgs.into_iter().enumerate() {
        let executor_cfg = ExecutorConfig {
            gossip,
            server_addr: with_unspecified_ip(addrs[i]),
            genesis_block: genesis.clone(),
            validators: validator_set.clone(),
        };
        let node_cfg = NodeConfig {
            executor: executor_cfg,
            metrics_server_addr,
            consensus: Some(ConsensusConfig {
                key: validator_keys[i].public(),
                public_addr: addrs[i],
            }),
        };

        // Recreate the directory for the node's config.
        let root = args.output_dir.join(addrs[i].to_string());
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).with_context(|| format!("create_dir_all({:?})", root))?;

        fs::write(
            root.join("config.json"),
            zksync_protobuf::encode_json(&node_cfg),
        )
        .context("fs::write()")?;
        fs::write(
            root.join("validator_key"),
            &TextFmt::encode(&validator_keys[i]),
        )
        .context("fs::write()")?;
        fs::write(root.join("node_key"), &TextFmt::encode(&node_keys[i])).context("fs::write()")?;
    }
    Ok(())
}
