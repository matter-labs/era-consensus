//! Library files for the executor. We have it separate from the binary so that we can use these files in the tools crate.
use anyhow::Context as _;
pub use network::{gossip::attestation, RpcConfig};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use zksync_concurrency::{ctx, limiter, net, scope, time};
use zksync_consensus_bft as bft;
use zksync_consensus_network as network;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{BlockStore, ReplicaStore};
use zksync_protobuf::kB;

#[cfg(test)]
mod tests;

/// Validator-related part of [`Executor`].
#[derive(Debug)]
pub struct Validator {
    /// Consensus network configuration.
    pub key: validator::SecretKey,
    /// Store for replica state.
    pub replica_store: Box<dyn ReplicaStore>,
    /// Payload manager.
    pub payload_manager: Box<dyn bft::PayloadManager>,
}

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
    pub view_timeout: usize,
    /// Key of this node. It uniquely identifies the node.
    /// It should match the secret key provided in the `node_key` file.
    pub node_key: node::SecretKey,
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
    /// Block storage used by the node.
    pub block_store: Arc<BlockStore>,
    /// Validator-specific node data.
    pub validator: Option<Validator>,
    /// Attestation controller. Caller should actively configure the batch
    /// for which the attestation votes should be collected.
    pub attestation: Arc<attestation::Controller>,
}

impl Executor {
    /// Extracts a network crate config.
    fn network_config(&self) -> network::Config {
        network::Config {
            build_version: self.config.build_version.clone(),
            server_addr: net::tcp::ListenerAddr::new(self.config.server_addr),
            public_addr: self.config.public_addr.clone(),
            gossip: self.config.gossip(),
            validator_key: self.validator.as_ref().map(|v| v.key.clone()),
            ping_timeout: Some(time::Duration::seconds(10)),
            max_block_size: self.config.max_payload_size.saturating_add(kB),
            max_block_queue_size: 20,
            tcp_accept_rate: limiter::Rate {
                burst: 10,
                refresh: time::Duration::milliseconds(100),
            },
            rpc: self.config.rpc.clone(),
            enable_pregenesis: true,
        }
    }

    /// Runs this executor to completion. This should be spawned on a separate task.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let network_config = self.network_config();

        // Generate the communication channels. We have one for each component.
        let (consensus_send, consensus_recv) = bft::create_input_channel();
        let (network_send, network_recv) = ctx::channel::unbounded();

        tracing::debug!("Starting components in separate threads.");
        scope::run!(ctx, |ctx, s| async {
            let (net, runner) = network::Network::new(
                network_config,
                self.block_store.clone(),
                consensus_send,
                network_recv,
                self.attestation,
            );
            net.register_metrics();
            s.spawn(async { runner.run(ctx).await.context("Network stopped") });

            if let Some(cfg) = self.config.debug_page {
                s.spawn(async {
                    network::debug_page::Server::new(cfg, net)
                        .run(ctx)
                        .await
                        .context("Debug page server stopped")
                });
            }

            // Run the bft component iff this node is an active validator.
            let Some(validator) = self.validator else {
                tracing::info!("Running the node in non-validator mode.");
                return Ok(());
            };
            if !self
                .block_store
                .genesis()
                .validators
                .contains(&validator.key.public())
            {
                tracing::warn!(
                    "This node is an inactive validator. It will NOT vote in consensus."
                );
                return Ok(());
            }
            bft::Config {
                secret_key: validator.key.clone(),
                block_store: self.block_store.clone(),
                replica_store: validator.replica_store,
                payload_manager: validator.payload_manager,
                max_payload_size: self.config.max_payload_size,
                view_timeout: time::Duration::milliseconds(self.config.view_timeout as i64),
            }
            .run(ctx, network_send, consensus_recv)
            .await
            .context("Consensus stopped")
        })
        .await
    }
}
