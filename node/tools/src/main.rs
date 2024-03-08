//! Main binary for the consensus node. It reads the configuration, initializes all parts of the node and
//! manages communication between the actors. It is the main executable in this workspace.
use anyhow::Context as _;
use clap::Parser;
use std::{fs, io::IsTerminal as _, path::PathBuf};
use tracing::metadata::LevelFilter;
use tracing_subscriber::{prelude::*, Registry};
use vise_exporter::MetricsExporter;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_tools::{decode_json, ConfigPaths, NodeAddr, RPCServer};
use zksync_protobuf::serde::Serde;

/// Wrapper for Vec<NodeAddr>.
#[derive(Debug, Clone)]
struct NodeAddrs(Vec<Serde<NodeAddr>>);

impl std::str::FromStr for NodeAddrs {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(decode_json(s)?))
    }
}

/// Command-line application launching a node executor.
#[derive(Debug, Parser)]
struct Args {
    /// Path to a validator key file. If set to an empty string, validator key will not be read
    /// (i.e., a node will be initialized as a non-validator node).
    #[arg(long, default_value = "./validator_key")]
    validator_key: PathBuf,
    /// Path to a JSON file with node configuration.
    #[arg(long, default_value = "./config.json")]
    config_file: PathBuf,
    /// Path to a node key file.
    #[arg(long, default_value = "./node_key")]
    node_key: PathBuf,
    /// Path to the rocksdb database of the node.
    #[arg(long, default_value = "./database")]
    database: PathBuf,
    /// Port for the RPC server.
    #[arg(long)]
    rpc_port: Option<u16>,
    /// IP address and key of the seed peers.
    #[arg(long)]
    add_gossip_static_outbound: Option<NodeAddrs>,
}

impl Args {
    /// Extracts configuration paths from these args.
    fn config_paths(&self) -> ConfigPaths<'_> {
        ConfigPaths {
            app: &self.config_file,
            node_key: &self.node_key,
            validator_key: (!self.validator_key.as_os_str().is_empty())
                .then_some(&self.validator_key),
            database: &self.database,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Args = Args::parse();
    tracing::trace!(?args, "Starting node");
    let ctx = &ctx::root();

    // Create log file.
    fs::create_dir_all("logs/")?;
    let log_file = fs::File::create("logs/output.log")?;

    // Create the logger for stdout. This will produce human-readable logs for ERROR events.
    // To see logs for other events, set the RUST_LOG environment to the desired level.
    let stdout_log = tracing_subscriber::fmt::layer()
        .pretty()
        .with_ansi(std::env::var("NO_COLOR").is_err() && std::io::stdout().is_terminal())
        .with_file(false)
        .with_line_number(false)
        .with_filter(tracing_subscriber::EnvFilter::from_default_env());

    // Create the logger for the log file. This will produce machine-readable logs for
    // all events of level DEBUG or higher.
    let file_log = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(log_file)
        .with_filter(LevelFilter::DEBUG);

    // Create the subscriber. This will combine the two loggers.
    let subscriber = Registry::default().with(stdout_log).with(file_log);

    // Set the subscriber as the global default. This will cause all events in all threads
    // to be logged by the subscriber.
    tracing::subscriber::set_global_default(subscriber).unwrap();

    // Start the node.
    tracing::info!("Starting node.");

    // Load the config files.
    tracing::debug!("Loading config files.");
    let mut configs = args
        .config_paths()
        .load()
        .context("config_paths().load()")?;

    // if `PUBLIC_ADDR` env var is set, use it to override publicAddr in config
    configs.app.check_public_addr().context("Public Address")?;

    // Add gossipStaticOutbound pairs from cli to config
    if let Some(addrs) = args.add_gossip_static_outbound {
        configs
            .app
            .gossip_static_outbound
            .extend(addrs.0.into_iter().map(|e| (e.0.key, e.0.addr)));
    }

    let (executor, runner) = configs
        .make_executor(ctx)
        .await
        .context("configs.into_executor()")?;

    let mut rpc_addr = configs.app.public_addr;
    if let Some(port) = args.rpc_port {
        rpc_addr.set_port(port);
    } else {
        rpc_addr.set_port(rpc_addr.port() + 100);
    }

    // Create the RPC server with the executor's storage.
    let node_storage = executor.block_store.clone();
    let rpc_server = RPCServer::new(rpc_addr, node_storage);

    // Initialize the storage.
    scope::run!(ctx, |ctx, s| async {
        if let Some(addr) = &configs.app.metrics_server_addr {
            s.spawn_bg(async {
                MetricsExporter::default()
                    .with_graceful_shutdown(ctx.canceled())
                    .start(*addr)
                    .await?;
                Ok(())
            });
        }
        s.spawn_bg(runner.run(ctx));
        s.spawn(executor.run(ctx));
        s.spawn(rpc_server.run(ctx));
        Ok(())
    })
    .await
}
