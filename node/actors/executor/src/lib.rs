//! Library files for the executor. We have it separate from the binary so that we can use these files in the tools crate.

use crate::io::Dispatcher;
use anyhow::Context as _;
use concurrency::{ctx, ctx::channel, net, scope};
use consensus::Consensus;
use roles::{node, validator, validator::FinalBlock};
use std::{mem, sync::Arc};
use storage::{FallbackReplicaStateStore, ReplicaStateStore, WriteBlockStore};
use sync_blocks::SyncBlocks;
use utils::pipe;

mod config;
mod io;
mod metrics;
#[cfg(test)]
mod tests;

pub use self::config::{ConsensusConfig, ExecutorConfig, GossipConfig};

/// Validator-related part of [`Executor`].
#[derive(Debug)]
struct ValidatorExecutor {
    /// Consensus network configuration.
    config: ConsensusConfig,
    /// Validator key.
    key: validator::SecretKey,
    /// Store for replica state.
    replica_state_store: Arc<dyn ReplicaStateStore>,
    /// Sender of blocks finalized by the consensus algorithm.
    blocks_sender: channel::UnboundedSender<FinalBlock>,
}

impl ValidatorExecutor {
    /// Returns consensus network configuration.
    fn consensus_config(&self) -> network::consensus::Config {
        network::consensus::Config {
            // Consistency of the validator key has been verified in constructor.
            key: self.key.clone(),
            public_addr: self.config.public_addr,
        }
    }
}

/// Executor allowing to spin up all actors necessary for a consensus node.
#[derive(Debug)]
pub struct Executor<S> {
    /// General-purpose executor configuration.
    executor_config: ExecutorConfig,
    /// Secret key of the node.
    node_key: node::SecretKey,
    /// Block and replica state storage used by the node.
    storage: Arc<S>,
    /// Validator-specific node data.
    validator: Option<ValidatorExecutor>,
}

impl<S: WriteBlockStore + 'static> Executor<S> {
    /// Creates a new executor with the specified parameters.
    pub fn new(
        node_config: ExecutorConfig,
        node_key: node::SecretKey,
        storage: Arc<S>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            node_config.gossip.key == node_key.public(),
            "config.gossip.key = {:?} doesn't match the secret key {:?}",
            node_config.gossip.key,
            node_key
        );

        Ok(Self {
            executor_config: node_config,
            node_key,
            storage,
            validator: None,
        })
    }

    /// Sets validator-related data for the executor.
    pub fn set_validator(
        &mut self,
        config: ConsensusConfig,
        key: validator::SecretKey,
        replica_state_store: Arc<dyn ReplicaStateStore>,
        blocks_sender: channel::UnboundedSender<FinalBlock>,
    ) -> anyhow::Result<()> {
        let public = &config.key;
        anyhow::ensure!(
            *public == key.public(),
            "config.consensus.key = {public:?} doesn't match the secret key {key:?}"
        );
        anyhow::ensure!(
            self.executor_config
                .validators
                .iter()
                .any(|validator_key| validator_key == public),
            "Key {public:?} is not a validator per validator set {:?}",
            self.executor_config.validators
        );
        self.validator = Some(ValidatorExecutor {
            config,
            key,
            replica_state_store,
            blocks_sender,
        });
        Ok(())
    }

    /// Returns gossip network configuration.
    fn gossip_config(&self) -> network::gossip::Config {
        let gossip = &self.executor_config.gossip;
        network::gossip::Config {
            key: self.node_key.clone(),
            dynamic_inbound_limit: gossip.dynamic_inbound_limit,
            static_inbound: gossip.static_inbound.clone(),
            static_outbound: gossip.static_outbound.clone(),
            enable_pings: true,
        }
    }

    /// Extracts a network crate config.
    fn network_config(&self) -> network::Config {
        network::Config {
            server_addr: net::tcp::ListenerAddr::new(self.executor_config.server_addr),
            validators: self.executor_config.validators.clone(),
            gossip: self.gossip_config(),
            consensus: self
                .validator
                .as_ref()
                .map(ValidatorExecutor::consensus_config),
        }
    }

    /// Runs this executor to completion. This should be spawned on a separate task.
    pub async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let network_config = self.network_config();

        // Generate the communication pipes. We have one for each actor.
        let (consensus_actor_pipe, consensus_dispatcher_pipe) = pipe::new();
        let (sync_blocks_actor_pipe, sync_blocks_dispatcher_pipe) = pipe::new();
        let (network_actor_pipe, network_dispatcher_pipe) = pipe::new();
        let blocks_sender = if let Some(validator) = &mut self.validator {
            mem::replace(&mut validator.blocks_sender, channel::unbounded().0)
        } else {
            channel::unbounded().0
        };
        // Create the IO dispatcher.
        let mut dispatcher = Dispatcher::new(
            consensus_dispatcher_pipe,
            sync_blocks_dispatcher_pipe,
            network_dispatcher_pipe,
            blocks_sender,
        );

        // Create each of the actors.
        let validator_set = &self.executor_config.validators;
        let consensus = if let Some(validator) = self.validator {
            let consensus_storage =
                FallbackReplicaStateStore::new(validator.replica_state_store, self.storage.clone());
            let consensus = Consensus::new(
                ctx,
                consensus_actor_pipe,
                validator.key.clone(),
                validator_set.clone(),
                consensus_storage,
            )
            .await
            .context("consensus")?;
            Some(consensus)
        } else {
            None
        };

        let sync_blocks_config = sync_blocks::Config::new(
            validator_set.clone(),
            consensus::misc::consensus_threshold(validator_set.len()),
        )?;
        let sync_blocks = SyncBlocks::new(
            ctx,
            sync_blocks_actor_pipe,
            self.storage,
            sync_blocks_config,
        )
        .await
        .context("sync_blocks")?;

        let sync_blocks_subscriber = sync_blocks.subscribe_to_state_updates();

        tracing::debug!("Starting actors in separate threads.");
        scope::run!(ctx, |ctx, s| async {
            s.spawn_blocking(|| dispatcher.run(ctx).context("IO Dispatcher stopped"));
            s.spawn(async {
                let state = network::State::new(network_config, None, Some(sync_blocks_subscriber));
                state.register_metrics();
                network::run_network(ctx, state, network_actor_pipe)
                    .await
                    .context("Network stopped")
            });
            if let Some(consensus) = consensus {
                s.spawn_blocking(|| consensus.run(ctx).context("Consensus stopped"));
            }
            sync_blocks.run(ctx).await.context("Syncing blocks stopped")
        })
        .await
    }
}