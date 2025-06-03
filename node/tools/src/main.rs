//! Main binary for the consensus node. It reads the configuration, initializes all parts of the node and
//! manages communication between the components. It is the main executable in this workspace.
use std::{fs, fs::Permissions, io::IsTerminal as _, os::unix::fs::PermissionsExt, path::PathBuf};

use anyhow::Context as _;
use clap::Parser;
use tracing::metadata::LevelFilter;
use tracing_subscriber::{prelude::*, Registry};
use vise_exporter::MetricsExporter;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_tools::{config, RPCServer};
use zksync_protobuf::serde::Deserialize;

/// Command-line application launching a node executor.
#[derive(Debug, Parser)]
struct Cli {
    /// Path to the file with json config.
    #[arg(long, default_value = "./config.json")]
    config_path: PathBuf,
    /// Inlined json config.
    #[arg(long, conflicts_with = "config_path")]
    config: Option<String>,
    /// Path to the rocksdb database of the node.
    /// If not provided, an in-memory database will be used instead.
    #[arg(long)]
    database: Option<PathBuf>,
}

impl Cli {
    /// Extracts configuration from the cli args.
    fn load(&self) -> anyhow::Result<config::Configs> {
        let json = match &self.config {
            Some(json) => json.clone(),
            None => fs::read_to_string(&self.config_path)?,
        };
        Ok(config::Configs {
            app: Deserialize {
                deny_unknown_fields: true,
            }
            .proto_fmt_from_json(&json)?,
            database: self.database.clone(),
        })
    }
}

/// Overrides `cfg.public_addr`, based on the `PUBLIC_ADDR` env variable.
fn check_public_addr(cfg: &mut config::App) -> anyhow::Result<()> {
    if let Ok(public_addr) = std::env::var("PUBLIC_ADDR") {
        cfg.public_addr =
            std::net::SocketAddr::new(public_addr.parse()?, config::NODES_PORT).into();
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Cli = Cli::parse();
    let ctx = &ctx::root();
    tracing::trace!("Starting node");

    // Create log file.
    let dir_path = "logs/";
    fs::create_dir_all(dir_path)?;
    fs::set_permissions(dir_path, Permissions::from_mode(0o700))?;

    let file_path = "logs/output.log";
    let log_file = fs::File::create(file_path)?;
    fs::set_permissions(file_path, Permissions::from_mode(0o600))?;

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
    tracing::trace!("Starting node.");

    // Load the config files.
    tracing::trace!("Loading config files.");
    let mut configs = args.load().context("config_args().load()")?;
    // if `PUBLIC_ADDR` env var is set, use it to override publicAddr in config
    check_public_addr(&mut configs.app).context("check_public_addr()")?;

    scope::run!(ctx, |ctx, s| async {
        let (executor, runner) = configs
            .make_executor(ctx)
            .await
            .context("configs.into_executor()")?;
        s.spawn_bg(runner.run(ctx));
        if let Some(addr) = &configs.app.metrics_server_addr {
            s.spawn_bg(async {
                MetricsExporter::default()
                    .with_graceful_shutdown(ctx.canceled())
                    .start(*addr)
                    .await?;
                Ok(())
            });
        }
        if let Some(rpc_addr) = &configs.app.rpc_addr {
            s.spawn_bg(RPCServer::new(*rpc_addr, executor.engine_manager.clone()).run(ctx));
        }
        executor.run(ctx).await
    })
    .await
}
