//! The inner data of the consensus state machine. This is shared between the different roles.
use crate::{PayloadManager};
use std::sync::Arc;
use tracing::instrument;
use zksync_consensus_roles::validator;
use zksync_consensus_storage as storage;

/// Configuration of the bft actor.
#[derive(Debug)]
pub struct Config {
    /// The validator's secret key.
    pub secret_key: validator::SecretKey,
    /// Genesis
    pub genesis: validator::Genesis,
    /// The maximum size of the payload of a block, in bytes. We will
    /// reject blocks with payloads larger than this.
    pub max_payload_size: usize,
    /// Block store.
    pub block_store: Arc<storage::BlockStore>,
    /// Replica store.
    pub replica_store: Box<dyn storage::ReplicaStore>,
    /// Payload manager.
    pub payload_manager: Box<dyn PayloadManager>,
}
