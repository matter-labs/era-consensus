//! The inner data of the consensus state machine. This is shared between the different roles.
use std::sync::Arc;

use anyhow::Context as _;
use zksync_concurrency::time;
use zksync_consensus_engine::EngineManager;
use zksync_consensus_roles::validator;

/// Configuration of the bft component.
#[derive(Debug)]
pub struct Config {
    /// Engine manager.
    pub engine_manager: Arc<EngineManager>,
    /// The validator's secret key.
    pub secret_key: validator::SecretKey,
    /// The maximum size of the payload of a block, in bytes. We will
    /// reject blocks with payloads larger than this.
    pub max_payload_size: usize,
    /// The duration of the view timeout.
    pub view_timeout: time::Duration,
    /// The epoch number for this BFT instance.
    pub epoch: validator::EpochNumber,
    /// The validator schedule for this epoch. We cache it here to avoid
    /// recomputing it on every call.
    validators: validator::Schedule,
}

impl Config {
    /// Creates a new config.
    pub fn new(
        secret_key: validator::SecretKey,
        max_payload_size: usize,
        view_timeout: time::Duration,
        engine_manager: Arc<EngineManager>,
        epoch_number: validator::EpochNumber,
    ) -> anyhow::Result<Self> {
        let validators = engine_manager
            .validator_schedule(epoch_number)
            .context(format!(
                "BFT config can't be created for epoch {} because there's no corresponding validator schedule.",
                epoch_number,
            ))?
            .schedule;

        Ok(Self {
            engine_manager,
            secret_key,
            max_payload_size,
            view_timeout,
            epoch: epoch_number,
            validators,
        })
    }

    /// Genesis hash.
    pub(crate) fn genesis_hash(&self) -> validator::GenesisHash {
        self.engine_manager.genesis_hash()
    }

    /// Protocol version of the genesis.
    pub(crate) fn protocol_version(&self) -> validator::ProtocolVersion {
        self.engine_manager.protocol_version()
    }

    /// Genesis first block.
    pub(crate) fn first_block(&self) -> validator::BlockNumber {
        self.engine_manager.first_block()
    }

    /// Validator schedule for this epoch.
    pub(crate) fn validators(&self) -> &validator::Schedule {
        &self.validators
    }
}
