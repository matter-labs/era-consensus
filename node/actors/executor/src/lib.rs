//! Library files for the executor. We have it separate from the binary so that we can use these files in the tools crate.
use crate::io::Dispatcher;
use anyhow::Context as _;
use network::http;
pub use network::RpcConfig;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use zksync_concurrency::{ctx, limiter, net, scope, time};
use zksync_consensus_bft as bft;
use zksync_consensus_network::{self as network, gossip::AttestationStatusWatch};
use zksync_consensus_roles::{attester, node, validator};
use zksync_consensus_storage::{BatchStore, BlockStore, ReplicaStore};
use zksync_consensus_utils::pipe;
use zksync_protobuf::kB;

mod attestation;
mod io;
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

/// Validator-related part of [`Executor`].
#[derive(Debug)]
pub struct Attester {
    /// Consensus network configuration.
    pub key: attester::SecretKey,
}

/// Config of the node executor.
#[derive(Debug)]
pub struct Config {
    /// IP:port to listen on, for incoming TCP connections.
    /// Use `0.0.0.0:<port>` to listen on all network interfaces (i.e. on all IPs exposed by this VM).
    pub server_addr: std::net::SocketAddr,
    /// Public TCP address that other nodes are expected to connect to.
    /// It is announced over gossip network.
    pub public_addr: net::Host,
    /// Maximal size of the block payload.
    pub max_payload_size: usize,
    /// Maximal size of a batch, which includes `max_payload_size` per block in the batch,
    /// plus the size of the Merkle proof of the commitment being included on L1.
    pub max_batch_size: usize,
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
    pub debug_page: Option<http::DebugPageConfig>,
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

/// Executor allowing to spin up all actors necessary for a consensus node.
#[derive(Debug)]
pub struct Executor {
    /// General-purpose executor configuration.
    pub config: Config,
    /// Block storage used by the node.
    pub block_store: Arc<BlockStore>,
    /// Batch storage used by the node.
    pub batch_store: Arc<BatchStore>,
    /// Validator-specific node data.
    pub validator: Option<Validator>,
    /// Validator-specific node data.
    pub attester: Option<Attester>,
    /// Status showing where the main node wants attester to cast their votes.
    pub attestation_status: Arc<AttestationStatusWatch>,
}

impl Executor {
    /// Extracts a network crate config.
    fn network_config(&self) -> network::Config {
        network::Config {
            server_addr: net::tcp::ListenerAddr::new(self.config.server_addr),
            public_addr: self.config.public_addr.clone(),
            gossip: self.config.gossip(),
            validator_key: self.validator.as_ref().map(|v| v.key.clone()),
            attester_key: self.attester.as_ref().map(|a| a.key.clone()),
            ping_timeout: Some(time::Duration::seconds(10)),
            max_block_size: self.config.max_payload_size.saturating_add(kB),
            max_batch_size: self.config.max_batch_size.saturating_add(kB),
            max_block_queue_size: 20,
            tcp_accept_rate: limiter::Rate {
                burst: 10,
                refresh: time::Duration::milliseconds(100),
            },
            rpc: self.config.rpc.clone(),
        }
    }

    /// Runs this executor to completion. This should be spawned on a separate task.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let network_config = self.network_config();

        // Generate the communication pipes. We have one for each actor.
        let (consensus_actor_pipe, consensus_dispatcher_pipe) = pipe::new();
        let (network_actor_pipe, network_dispatcher_pipe) = pipe::new();
        // Create the IO dispatcher.
        let dispatcher = Dispatcher::new(consensus_dispatcher_pipe, network_dispatcher_pipe);

        tracing::debug!("Starting actors in separate threads.");
        scope::run!(ctx, |ctx, s| async {
            s.spawn(async { dispatcher.run(ctx).await.context("IO Dispatcher stopped") });
            let (net, runner) = network::Network::new(
                network_config,
                self.block_store.clone(),
                self.batch_store.clone(),
                network_actor_pipe,
                self.attestation_status.clone(),
            );
            net.register_metrics();
            s.spawn(async { runner.run(ctx).await.context("Network stopped") });

            if let Some(attester) = self.attester {
                tracing::info!("Running the node in attester mode.");
                let runner = attestation::AttesterRunner::new(
                    self.block_store.clone(),
                    self.batch_store.clone(),
                    attester,
                    net.batch_vote_publisher(),
                    self.attestation_status.subscribe(),
                );
                s.spawn(async {
                    runner.run(ctx).await?;
                    Ok(())
                });
            } else {
                tracing::info!("Running the node in non-attester mode.");
            }

            if let Some(debug_config) = self.config.debug_page {
                s.spawn(async {
                    http::DebugPageServer::new(debug_config, net)
                        .run(ctx)
                        .await
                        .context("Http Server stopped")
                });
            }

            // Run the bft actor iff this node is an active validator.
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
            }
            .run(ctx, consensus_actor_pipe)
            .await
            .context("Consensus stopped")
        })
        .await
    }
}
