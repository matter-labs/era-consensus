//! The inner data of the consensus state machine. This is shared between the different roles.
use crate::PayloadManager;
use std::sync::Arc;
use zksync_concurrency::time;
use zksync_consensus_roles::validator;
use zksync_consensus_storage as storage;

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
    /// Block store.
    pub block_store: Arc<storage::BlockStore>,
    /// Replica store.
    pub replica_store: Box<dyn storage::ReplicaStore>,
    /// Payload manager.
    pub payload_manager: Box<dyn PayloadManager>,
}

impl Config {
    /// Genesis.
    pub fn genesis(&self) -> &validator::Genesis {
        self.block_store.genesis()
    }
}
