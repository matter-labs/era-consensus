//! Main binary for the consensus node. It reads the configuration, initializes all parts of the node and
//! manages communication between the actors. It is the main executable in this workspace.
use anyhow::Context as _;
use clap::Parser;
use std::{error::Error, fs, io::IsTerminal as _, path::PathBuf};
use tracing::metadata::LevelFilter;
use tracing_subscriber::{prelude::*, Registry};
use vise_exporter::MetricsExporter;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_crypto::read_required_text;
use zksync_consensus_roles::node;
use zksync_consensus_tools::ConfigPaths;
use zksync_consensus_utils::no_copy::NoCopy;

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
    #[arg(long="seed", value_parser = parse_seed_peer)]
    seed_peers: Vec<(std::net::SocketAddr, node::PublicKey)>,
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

/// Parse a single (SocketAddr,node key) pair
fn parse_seed_peer(
    s: &str,
) -> Result<(std::net::SocketAddr, node::PublicKey), Box<dyn Error + Send + Sync>> {
    let pos = s
        .find(',')
        .ok_or_else(|| "expected format: <IP>:<Port>,<key>")?;
    Ok((
        s[..pos].parse()?,
        read_required_text(&Some(s[pos + 1..].parse()?))?,
    ))
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
    configs.app.add_seed_peers(args.seed_peers);

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
