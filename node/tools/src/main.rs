//! Main binary for the consensus node. It reads the configuration, initializes all parts of the node and
//! manages communication between the actors. It is the main executable in this workspace.
use anyhow::Context as _;
use clap::Parser;
use std::{fs, io::IsTerminal as _, path::PathBuf, str::FromStr};
use tracing::metadata::LevelFilter;
use tracing_subscriber::{prelude::*, Registry};
use vise_exporter::MetricsExporter;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_crypto::read_required_text;
use zksync_consensus_roles::node;
use zksync_consensus_tools::ConfigPaths;
use zksync_consensus_utils::no_copy::NoCopy;

/// Utility struct to parse json value from cli arg
#[derive(Debug, Clone)]
struct NodeAddresses(Vec<(node::PublicKey, std::net::SocketAddr)>);

impl FromStr for NodeAddresses {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value: serde_json::Value =
            serde_json::from_str(s).map_err(|e| format!("Malformed json: {}", e))?;
        let array = value.as_array().ok_or("Array expected")?;
        let result = array
            .iter()
            .map(|e| {
                let key = read_required_text(&e["key"].as_str().map(|s| s.to_string()))
                    .expect("Invalid key");
                let addr = read_required_text(&e["addr"].as_str().map(|s| s.to_string()))
                    .expect("Invalid address");
                (key, addr)
            })
            .collect();
        Ok(NodeAddresses(result))
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
    /// IP address and key of the seed peers.
    #[arg(long = "add_gossip_static_outbound")]
    gossip_static_outbound: NodeAddresses,
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

    // Add gossipStaticOutbound pairs from cli to config
    configs
        .app
        .gossip_static_outbound
        .extend(args.gossip_static_outbound.0);

    let (executor, runner) = configs
        .make_executor(ctx)
        .await
        .context("configs.into_executor()")?;

    // Initialize the storage.
    scope::run!(ctx, |ctx, s| async {
        if let Some(addr) = configs.app.metrics_server_addr {
            let addr = NoCopy::from(addr);
            s.spawn_bg(async {
                let addr = addr;
                MetricsExporter::default()
                    .with_graceful_shutdown(ctx.canceled())
                    .start(*addr)
                    .await?;
                Ok(())
            });
        }
        s.spawn_bg(runner.run(ctx));
        s.spawn(executor.run(ctx));
        Ok(())
    })
    .await
}
