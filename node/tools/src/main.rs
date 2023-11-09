//! Main binary for the consensus node. It reads the configuration, initializes all parts of the node and
//! manages communication between the actors. It is the main executable in this workspace.
use anyhow::Context as _;
use clap::Parser;
use std::{
    fs,
    io::IsTerminal as _,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::metadata::LevelFilter;
use tracing_subscriber::{prelude::*, Registry};
use vise_exporter::MetricsExporter;
use zksync_concurrency::{
    ctx::{self, channel},
    scope, time,
};
use zksync_consensus_executor::Executor;
use zksync_consensus_storage::{BlockStore, RocksdbStorage};
use zksync_consensus_tools::{ConfigPaths, Configs};
use zksync_consensus_utils::no_copy::NoCopy;

/// Command-line application launching a node executor.
#[derive(Debug, Parser)]
struct Args {
    /// Verify configuration instead of launching a node.
    #[arg(long, conflicts_with_all = ["ci_mode", "validator_key", "config_file", "node_key"])]
    verify_config: bool,
    /// Exit after finalizing 100 blocks.
    #[arg(long)]
    ci_mode: bool,
    /// Path to a validator key file. If set to an empty string, validator key will not be read
    /// (i.e., a node will be initialized as a non-validator node).
    #[arg(long, default_value = "validator_key")]
    validator_key: PathBuf,
    /// Path to a JSON file with node configuration.
    #[arg(long, default_value = "config.json")]
    config_file: PathBuf,
    /// Path to a node key file.
    #[arg(long, default_value = "node_key")]
    node_key: PathBuf,
}

impl Args {
    /// Extracts configuration paths from these args.
    fn config_paths(&self) -> ConfigPaths<'_> {
        ConfigPaths {
            config: &self.config_file,
            node_key: &self.node_key,
            validator_key: (!self.validator_key.as_os_str().is_empty())
                .then_some(&self.validator_key),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Args = Args::parse();
    tracing::trace!(?args, "Starting node");
    let ctx = &ctx::root();

    if !args.verify_config {
        // Create log file.
        fs::create_dir_all("logs/")?;
        let log_file = fs::File::create("logs/output.log")?;

        // Create the logger for stdout. This will produce human-readable logs for
        // all events of level INFO or higher.
        let stdout_log = tracing_subscriber::fmt::layer()
            .pretty()
            .with_ansi(std::env::var("NO_COLOR").is_err() && std::io::stdout().is_terminal())
            .with_file(false)
            .with_line_number(false)
            .with_filter(LevelFilter::INFO);

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
    }

    // Load the config files.
    tracing::debug!("Loading config files.");
    let configs = Configs::read(args.config_paths()).context("configs.read()")?;

    if args.verify_config {
        tracing::info!("Configuration verified.");
        return Ok(());
    }

    // Initialize the storage.
    tracing::debug!("Initializing storage.");

    let storage = RocksdbStorage::new(
        ctx,
        &configs.executor.genesis_block,
        Path::new("./database"),
    );
    let storage = Arc::new(storage.await.context("RocksdbStorage::new()")?);
    let mut executor = Executor::new(configs.executor, configs.node_key, storage.clone())
        .context("Executor::new()")?;
    if let Some((consensus_config, validator_key)) = configs.consensus {
        let blocks_sender = channel::unbounded().0; // Just drop finalized blocks
        executor
            .set_validator(
                consensus_config,
                validator_key,
                storage.clone(),
                blocks_sender,
            )
            .context("Executor::set_validator()")?;
    }

    scope::run!(ctx, |ctx, s| async {
        if let Some(addr) = configs.metrics_server_addr {
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

        s.spawn(executor.run(ctx));

        // if we are in CI mode, we wait for the node to finalize 100 blocks and then we stop it
        if args.ci_mode {
            let storage = storage.clone();
            loop {
                let block_finalized = storage.head_block(ctx).await.context("head_block")?;
                let block_finalized = block_finalized.header.number.0;

                tracing::info!("current finalized block {}", block_finalized);
                if block_finalized > 100 {
                    // we wait for 10 seconds to make sure that we send enough messages to other nodes
                    // and other nodes have enough messages to finalize 100+ blocks
                    ctx.sleep(time::Duration::seconds(10)).await?;
                    break;
                }
                ctx.sleep(time::Duration::seconds(1)).await?;
            }

            tracing::info!("Cancel all tasks");
            s.cancel();
        }
        Ok(())
    })
    .await
    .context("node stopped")
}
