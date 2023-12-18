//! Library files for the executor. We have it separate from the binary so that we can use these files in the tools crate.
use crate::io::Dispatcher;
use anyhow::Context as _;
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};
use zksync_concurrency::{ctx, net, scope};
use zksync_consensus_bft::{misc::consensus_threshold, PayloadSource};
use zksync_consensus_network as network;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{ReplicaStateStore, ReplicaStore, WriteBlockStore};
use zksync_consensus_sync_blocks::SyncBlocks;
use zksync_consensus_utils::pipe;

mod io;
pub mod testonly;
#[cfg(test)]
mod tests;

pub use network::consensus::Config as ValidatorConfig;

/// Validator-related part of [`Executor`].
pub struct Validator {
    /// Consensus network configuration.
    pub config: ValidatorConfig,
    /// Store for replica state.
    pub replica_state_store: Arc<dyn ReplicaStateStore>,
    /// Payload proposer for new blocks.
    pub payload_source: Arc<dyn PayloadSource>,
}

impl fmt::Debug for Validator {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("ValidatorExecutor")
            .field("config", &self.config)
            .finish()
    }
}

/// Config of the node executor.
#[derive(Clone, Debug)]
pub struct Config {
    /// IP:port to listen on, for incoming TCP connections.
    /// Use `0.0.0.0:<port>` to listen on all network interfaces (i.e. on all IPs exposed by this VM).
    pub server_addr: std::net::SocketAddr,
    /// Static specification of validators for Proof of Authority. Should be deprecated once we move
    /// to Proof of Stake.
    pub validators: validator::ValidatorSet,

    /// Key of this node. It uniquely identifies the node.
    /// It should match the secret key provided in the `node_key` file.
    pub node_key: node::SecretKey,
    /// Limit on the number of inbound connections outside
    /// of the `static_inbound` set.
    pub gossip_dynamic_inbound_limit: u64,
    /// Inbound connections that should be unconditionally accepted.
    pub gossip_static_inbound: HashSet<node::PublicKey>,
    /// Outbound connections that the node should actively try to
    /// establish and maintain.
    pub gossip_static_outbound: HashMap<node::PublicKey, std::net::SocketAddr>,
}

impl Config {
    /// Returns gossip network configuration.
    pub(crate) fn gossip(&self) -> network::gossip::Config {
        network::gossip::Config {
            key: self.node_key.clone(),
            dynamic_inbound_limit: self.gossip_dynamic_inbound_limit,
            static_inbound: self.gossip_static_inbound.clone(),
            static_outbound: self.gossip_static_outbound.clone(),
            enable_pings: true,
        }
    }
}

/// Executor allowing to spin up all actors necessary for a consensus node.
#[derive(Debug)]
pub struct Executor {
    /// General-purpose executor configuration.
    pub config: Config,
    /// Block and replica state storage used by the node.
    pub storage: Arc<dyn WriteBlockStore>,
    /// Validator-specific node data.
    pub validator: Option<Validator>,
}

impl Executor {
    /// Extracts a network crate config.
    fn network_config(&self) -> network::Config {
        network::Config {
            server_addr: net::tcp::ListenerAddr::new(self.config.server_addr),
            validators: self.config.validators.clone(),
            gossip: self.config.gossip(),
            consensus: self.active_validator().map(|v| v.config.clone()),
        }
    }

    /// Returns the validator setup <=> this node is a validator which belongs to
    /// the consensus (i.e. is in the `validators` set.
    fn active_validator(&self) -> Option<&Validator> {
        // TODO: this logic must be refactored once dynamic validator sets are implemented
        let validator = self.validator.as_ref()?;
        if self
            .config
            .validators
            .iter()
            .any(|key| key == &validator.config.key.public())
        {
            return Some(validator);
        }
        None
    }

    /// Runs this executor to completion. This should be spawned on a separate task.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let network_config = self.network_config();

        // Generate the communication pipes. We have one for each actor.
        let (consensus_actor_pipe, consensus_dispatcher_pipe) = pipe::new();
        let (sync_blocks_actor_pipe, sync_blocks_dispatcher_pipe) = pipe::new();
        let (network_actor_pipe, network_dispatcher_pipe) = pipe::new();
        // Create the IO dispatcher.
        let mut dispatcher = Dispatcher::new(
            consensus_dispatcher_pipe,
            sync_blocks_dispatcher_pipe,
            network_dispatcher_pipe,
        );

        // Create each of the actors.
        let validator_set = &self.config.validators;
        let sync_blocks_config = zksync_consensus_sync_blocks::Config::new(
            validator_set.clone(),
            consensus_threshold(validator_set.len()),
        )?;
        let sync_blocks = SyncBlocks::new(
            ctx,
            sync_blocks_actor_pipe,
            self.storage.clone(),
            sync_blocks_config,
        )
        .await
        .context("sync_blocks")?;

        let sync_blocks_subscriber = sync_blocks.subscribe_to_state_updates();

        tracing::debug!("Starting actors in separate threads.");
        scope::run!(ctx, |ctx, s| async {
            s.spawn_blocking(|| dispatcher.run(ctx).context("IO Dispatcher stopped"));
            s.spawn(async {
                let state = network::State::new(network_config, None, Some(sync_blocks_subscriber))
                    .context("Invalid network config")?;
                state.register_metrics();
                network::run_network(ctx, state, network_actor_pipe)
                    .await
                    .context("Network stopped")
            });
            if let Some(validator) = self.active_validator() {
                s.spawn(async {
                    let consensus_storage = ReplicaStore::new(
                        validator.replica_state_store.clone(),
                        self.storage.clone(),
                    );
                    zksync_consensus_bft::run(
                        ctx,
                        consensus_actor_pipe,
                        validator.config.key.clone(),
                        validator_set.clone(),
                        consensus_storage,
                        &*validator.payload_source,
                    )
                    .await
                    .context("Consensus stopped")
                });
            }
            sync_blocks.run(ctx).await.context("Syncing blocks stopped")
        })
        .await
    }
}
