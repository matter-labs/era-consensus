//! Main binary for the consensus node. It reads the configuration, initializes all parts of the node and
//! manages communication between the actors. It is the main executable in this workspace.

use anyhow::Context as _;
use concurrency::{ctx, scope, time};
use executor::Executor;
use std::{fs, io::IsTerminal as _, path::Path, sync::Arc};
use storage::{BlockStore, RocksdbStorage};
use tools::Configs;
use tracing::metadata::LevelFilter;
use tracing_subscriber::{prelude::*, Registry};
use utils::no_copy::NoCopy;
use vise_exporter::MetricsExporter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ctx = &ctx::root();

    // Get the command line arguments.
    let args: Vec<_> = std::env::args().collect();

    // Check if we are in config mode.
    let config_mode = args.iter().any(|x| x == "--verify-config");

    // Check if we are in CI mode.
    // If we are in CI mode, we will exit after finalizing more than 100 blocks.
    let ci_mode = args.iter().any(|x| x == "--ci-mode");

    if !config_mode {
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
    let configs = Configs::read(&args).context("configs.read()")?;

    if config_mode {
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
        executor
            .set_validator(consensus_config, validator_key)
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
        if ci_mode {
            let storage = storage.clone();
            loop {
                let block_finalized = storage.head_block(ctx).await.context("head_block")?;
                let block_finalized = block_finalized.block.number.0;

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
