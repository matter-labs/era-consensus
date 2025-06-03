//! Library files for the executor. We have it separate from the binary so that we can use these files in the tools crate.
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::Context as _;
pub use network::RpcConfig;
use zksync_concurrency::{ctx, error::Wrap as _, limiter, net, scope, time};
use zksync_consensus_bft as bft;
use zksync_consensus_engine::EngineManager;
use zksync_consensus_network as network;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::kB;

#[cfg(test)]
mod tests;

/// Config of the node executor.
#[derive(Debug)]
pub struct Config {
    /// Label identifying the build version of the binary that this node is running.
    pub build_version: Option<semver::Version>,
    /// IP:port to listen on, for incoming TCP connections.
    /// Use `0.0.0.0:<port>` to listen on all network interfaces (i.e. on all IPs exposed by this VM).
    pub server_addr: std::net::SocketAddr,
    /// Public TCP address that other nodes are expected to connect to.
    /// It is announced over gossip network.
    pub public_addr: net::Host,

    /// Maximal size of the block payload.
    pub max_payload_size: usize,
    /// The duration of the view timeout, in milliseconds.
    pub view_timeout: time::Duration,
    /// Limit on the number of inbound connections outside
    /// of the `static_inbound` set.
    pub gossip_dynamic_inbound_limit: usize,
    /// Inbound connections that should be unconditionally accepted.
    pub gossip_static_inbound: HashSet<node::PublicKey>,
    /// Outbound connections that the node should actively try to
    /// establish and maintain.
    pub gossip_static_outbound: HashMap<node::PublicKey, net::Host>,
    /// RPC rate limits config.
    /// Use `RpcConfig::default()` for defaults.
    pub rpc: RpcConfig,

    /// Node's secret key. It uniquely identifies the node in the gossip network.
    /// It should match the secret key provided in the `node_key` file.
    pub node_key: node::SecretKey,
    /// Validator's secret key. It uniquely identifies the validator in the validator network.
    /// If it is not provided, the node will not participate in consensus.
    pub validator_key: Option<validator::SecretKey>,

    /// Http debug page configuration.
    /// If None, debug page is disabled
    pub debug_page: Option<network::debug_page::Config>,
}

impl Config {
    /// Returns gossip network configuration.
    pub(crate) fn gossip(&self) -> network::GossipConfig {
        network::GossipConfig {
            key: self.node_key.clone(),
            dynamic_inbound_limit: self.gossip_dynamic_inbound_limit,
            static_inbound: self.gossip_static_inbound.clone(),
            static_outbound: self.gossip_static_outbound.clone(),
        }
    }
}

/// Executor allowing to spin up all components necessary for a consensus node.
#[derive(Debug)]
pub struct Executor {
    /// General-purpose executor configuration.
    pub config: Config,
    /// Engine manager. It is responsible for the connection to the execution layer.
    pub engine_manager: Arc<EngineManager>,
}

impl Executor {
    /// Runs this executor to completion. This should be spawned on a separate task.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let res = scope::run!(ctx, |ctx, s| async {
            // If we are before the first block in genesis, we need to spawn the components
            // to fetch the pre-genesis blocks.
            if self.engine_manager.head().next() < self.engine_manager.first_block() {
                s.spawn(async {
                    self.spawn_components(ctx, self.network_config(), None, None)
                        .await
                        .wrap("Components for pregenesis stopped")
                });
            }

            // Wait for the first epoch to be populated to get number of the first epoch.
            let mut cur_epoch = self
                .engine_manager
                .wait_until_epoch_schedule_populated(ctx)
                .await?;

            // Start the loop to spawn the components for each epoch.
            loop {
                // Wait for the validator schedule for the current epoch.
                let schedule = self
                    .engine_manager
                    .wait_for_validator_schedule(ctx, cur_epoch)
                    .await?
                    .schedule;

                // We need to wrap this in a NoCopy to be able to move into the spawned task.
                let epoch = ctx::NoCopy(cur_epoch);

                // Spawn the components for the current epoch.
                s.spawn(async {
                    let epoch = epoch.into();
                    self.spawn_components(ctx, self.network_config(), Some(epoch), Some(schedule))
                        .await
                        .wrap(format!("Components for epoch {} stopped", epoch))
                });

                // Increment the epoch counter.
                cur_epoch = cur_epoch.next();
            }
        })
        .await;

        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }

    /// Spawns the bft and network components for the given epoch.
    async fn spawn_components(
        &self,
        ctx: &ctx::Ctx,
        network_config: network::Config,
        epoch: Option<validator::EpochNumber>,
        validator_schedule: Option<validator::Schedule>,
    ) -> ctx::Result<()> {
        if let Some(epoch_number) = epoch {
            tracing::debug!("Spawning components for epoch {}", epoch_number);
        } else {
            tracing::debug!("Spawning components for pregenesis");
        }

        // Generate the communication channels. We have one for each component.
        let (consensus_send, consensus_recv) = bft::create_input_channel();
        let (network_send, network_recv) = ctx::channel::unbounded();

        scope::run!(ctx, |ctx, s| async {
            // Run the network component.
            tracing::debug!("Starting network component.");
            let (net, runner) = network::Network::new(
                network_config,
                self.engine_manager.clone(),
                epoch,
                consensus_send,
                network_recv,
            )?;
            net.register_metrics();
            s.spawn(async { runner.run(ctx).await.context("Network stopped") });

            if let Some(cfg) = self.config.debug_page.clone() {
                s.spawn(async {
                    network::debug_page::Server::new(cfg, net)
                        .run(ctx)
                        .await
                        .context("Debug page server stopped")
                });
            }

            // Run the bft component iff this node is an active validator for this epoch.
            if epoch.is_some()
                && validator_schedule.is_some()
                && self.config.validator_key.is_some()
                && validator_schedule
                    .as_ref()
                    .unwrap()
                    .contains(&self.config.validator_key.clone().unwrap().public())
            {
                tracing::debug!(
                    "This node is an active validator for epoch {}. Starting bft component.",
                    epoch.unwrap()
                );
                s.spawn(async {
                    bft::Config::new(
                        self.config.validator_key.clone().unwrap(),
                        self.config.max_payload_size,
                        self.config.view_timeout,
                        self.engine_manager.clone(),
                        epoch.unwrap(),
                    )?
                    .run(ctx, network_send, consensus_recv)
                    .await
                    .context("Consensus stopped")
                });
            } else {
                tracing::debug!(
                    "Running the node in non-validator mode for epoch {}.",
                    epoch.unwrap()
                );
            }

            Ok(())
        })
        .await?;

        Ok(())
    }

    /// Extracts a network crate config.
    fn network_config(&self) -> network::Config {
        network::Config {
            build_version: self.config.build_version.clone(),
            server_addr: net::tcp::ListenerAddr::new(self.config.server_addr),
            public_addr: self.config.public_addr.clone(),
            gossip: self.config.gossip(),
            validator_key: self.config.validator_key.clone(),
            ping_timeout: Some(time::Duration::seconds(10)),
            max_block_size: self.config.max_payload_size.saturating_add(kB),
            max_block_queue_size: 20,
            tcp_accept_rate: limiter::Rate {
                burst: 10,
                refresh: time::Duration::milliseconds(100),
            },
            rpc: self.config.rpc.clone(),
        }
    }
}
