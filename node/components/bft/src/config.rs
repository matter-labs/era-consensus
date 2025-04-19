//! The inner data of the consensus state machine. This is shared between the different roles.
use std::sync::Arc;

use zksync_concurrency::time;
use zksync_consensus_engine::EngineManager;
use zksync_consensus_roles::validator;

/// Configuration of the bft component.
#[derive(Debug)]
pub struct Config {
    /// The validator's secret key.
    pub secret_key: validator::SecretKey,
    /// The maximum size of the payload of a block, in bytes. We will
    /// reject blocks with payloads larger than this.
    pub max_payload_size: usize,
    /// The duration of the view timeout.
    pub view_timeout: time::Duration,
    /// Engine manager.
    pub engine_manager: Arc<EngineManager>,
}

impl Config {
    /// Genesis.
    pub fn genesis(&self) -> &validator::Genesis {
        self.engine_manager.genesis()
    }
}
