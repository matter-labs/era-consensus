//! Library files for the executor. We have it separate from the binary so that we can use these files in the tools crate.
use crate::io::Dispatcher;
use anyhow::Context as _;
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};
use zksync_concurrency::{ctx, net, scope, sync};
use zksync_consensus_bft as bft;
use zksync_consensus_network as network;
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{BlockStore, BlockStoreState, ReplicaStore};
use zksync_consensus_sync_blocks as sync_blocks;
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
    pub replica_store: Box<dyn ReplicaStore>,
    /// Payload manager.
    pub payload_manager: Box<dyn bft::PayloadManager>,
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
    /// Block storage used by the node.
    pub block_store: Arc<BlockStore>,
    /// Validator-specific node data.
    pub validator: Option<Validator>,
}

/// Converts BlockStoreState to isomorphic network::io::SyncState.
fn to_sync_state(state: BlockStoreState) -> network::io::SyncState {
    network::io::SyncState {
        first_stored_block: state.first,
        last_stored_block: state.last,
    }
}

impl Executor {
    /// Extracts a network crate config.
    fn network_config(&self) -> network::Config {
        network::Config {
            server_addr: net::tcp::ListenerAddr::new(self.config.server_addr),
            validators: self.config.validators.clone(),
            gossip: self.config.gossip(),
            consensus: self.validator.as_ref().map(|v| v.config.clone()),
        }
    }

    /// Verifies correctness of the Executor.
    fn verify(&self) -> anyhow::Result<()> {
        if let Some(validator) = self.validator.as_ref() {
            if !self
                .config
                .validators
                .iter()
                .any(|key| key == &validator.config.key.public())
            {
                anyhow::bail!("this validator doesn't belong to the consensus");
            }
        }
        Ok(())
    }

    /// Runs this executor to completion. This should be spawned on a separate task.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        self.verify().context("verify()")?;
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
        let validator_set = self.config.validators;
        let mut block_store_state = self.block_store.subscribe();
        let sync_state = sync::watch::channel(to_sync_state(block_store_state.borrow().clone())).0;

        tracing::debug!("Starting actors in separate threads.");
        scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(async {
                // Task forwarding changes from block_store_state to sync_state.
                // Alternatively we can make network depend on the storage directly.
                while let Ok(state) = sync::changed(ctx, &mut block_store_state).await {
                    sync_state.send_replace(to_sync_state(state.clone()));
                }
                Ok(())
            });

            s.spawn_blocking(|| dispatcher.run(ctx).context("IO Dispatcher stopped"));
            s.spawn(async {
                let state = network::State::new(network_config, None, Some(sync_state.subscribe()))
                    .context("Invalid network config")?;
                state.register_metrics();
                network::run_network(ctx, state, network_actor_pipe)
                    .await
                    .context("Network stopped")
            });
            if let Some(validator) = self.validator {
                s.spawn(async {
                    let validator = validator;
                    bft::Config {
                        secret_key: validator.config.key.clone(),
                        validator_set: validator_set.clone(),
                        block_store: self.block_store.clone(),
                        replica_store: validator.replica_store,
                        payload_manager: validator.payload_manager,
                    }
                    .run(ctx, consensus_actor_pipe)
                    .await
                    .context("Consensus stopped")
                });
            }
            sync_blocks::Config::new(
                validator_set.clone(),
                bft::misc::consensus_threshold(validator_set.len()),
            )?
            .run(ctx, sync_blocks_actor_pipe, self.block_store.clone())
            .await
            .context("Syncing blocks stopped")
        })
        .await
    }
}
