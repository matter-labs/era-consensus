//! Main binary for the consensus node. It reads the configuration, initializes all parts of the node and
//! manages communication between the actors. It is the main executable in this workspace.
use anyhow::Context as _;
use clap::Parser;
use std::{fs, io::IsTerminal as _, path::PathBuf};
use tracing::metadata::LevelFilter;
use tracing_subscriber::{prelude::*, Registry};
use vise_exporter::MetricsExporter;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_crypto::{Text, TextFmt};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_tools::{decode_json, AppConfig, ConfigArgs, ConfigSource, RPCServer};
use zksync_protobuf::serde::Serde;

/// Command-line application launching a node executor.
#[derive(Debug, Parser)]
struct Cli {
    /// Full json config
    #[arg(long,
          value_parser(parse_config),
          requires="node_key",
          conflicts_with_all=["config_file", "validator_key_file", "node_key_file"])]
    config: Option<Serde<AppConfig>>,
    /// Plain node key
    #[arg(long,
          value_parser(parse_key::<node::SecretKey>),
          requires="config",
          conflicts_with_all=["config_file", "validator_key_file", "node_key_file"])]
    node_key: Option<node::SecretKey>,
    /// Plain validator key
    #[arg(long,
          value_parser(parse_key::<validator::SecretKey>),
          requires_all=["config", "node_key"],
          conflicts_with_all=["config_file", "validator_key_file", "node_key_file"])]
    validator_key: Option<validator::SecretKey>,
    /// Path to a validator key file. If set to an empty string, validator key will not be read
    /// (i.e., a node will be initialized as a non-validator node).
    #[arg(long,
          default_value = "./validator_key",
          conflicts_with_all=["config", "validator_key", "node_key"])]
    validator_key_file: PathBuf,
    /// Path to a JSON file with node configuration.
    #[arg(long,
          default_value = "./config.json",
          conflicts_with_all=["config", "validator_key", "node_key"])]
    config_file: PathBuf,
    /// Path to a node key file.
    #[arg(long,
          default_value = "./node_key",
          conflicts_with_all=["config", "validator_key", "node_key"])]
    node_key_file: PathBuf,
    /// Path to the rocksdb database of the node.
    #[arg(long, default_value = "./database")]
    database: PathBuf,
    /// Port for the RPC server.
    #[arg(long)]
    rpc_port: Option<u16>,
}

/// Function to let clap parse the command line `config` argument
fn parse_config(val: &str) -> anyhow::Result<Serde<AppConfig>> {
    decode_json(val)
}

/// Node/validator key parser for clap
fn parse_key<T: TextFmt>(val: &str) -> anyhow::Result<T> {
    Text::new(val).decode().context("failed decoding key")
}

impl Cli {
    /// Extracts configuration paths from these args.
    fn config_args(&self) -> ConfigArgs<'_> {
        let config_args = match &self.config {
            Some(config) => ConfigSource::CliConfig {
                config: config.clone().0,
                node_key: self.node_key.clone().unwrap(), // node_key is present as it is enforced by clap rules
                validator_key: self.validator_key.clone(),
            },
            None => ConfigSource::PathConfig {
                config_file: &self.config_file,
                validator_key_file: &self.validator_key_file,
                node_key_file: &self.node_key_file,
            },
        };
        ConfigArgs {
            config_args,
            database: &self.database,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Cli = Cli::parse();
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
    let mut configs = args.config_args().load().context("config_args().load()")?;

    // if `PUBLIC_ADDR` env var is set, use it to override publicAddr in config
    configs.app.check_public_addr().context("Public Address")?;

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
