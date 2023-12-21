//! The inner data of the consensus state machine. This is shared between the different roles.
use crate::{misc, PayloadManager};
use std::sync::Arc;
use zksync_consensus_storage as storage;
use tracing::instrument;
use zksync_consensus_roles::validator;

/// Configuration of the bft actor.
#[derive(Debug)]
pub struct Config {
    /// The validator's secret key.
    pub secret_key: validator::SecretKey,
    /// A vector of public keys for all the validators in the network.
    pub validator_set: validator::ValidatorSet,
    /// Block store.
    pub block_store: Arc<storage::BlockStore>,
    /// Replica store.
    pub replica_store: Box<dyn storage::ReplicaStore>,
    /// Payload manager.
    pub payload_manager: Box<dyn PayloadManager>,
}

impl Config {
    /// The maximum size of the payload of a block, in bytes. We will
    /// reject blocks with payloads larger than this.
    pub(crate) const PAYLOAD_MAX_SIZE: usize = 500 * zksync_protobuf::kB;

    /// Computes the validator for the given view.
    #[instrument(level = "trace", ret)]
    pub fn view_leader(&self, view_number: validator::ViewNumber) -> validator::PublicKey {
        let index = view_number.0 as usize % self.validator_set.len();
        self.validator_set.get(index).unwrap().clone()
    }

    /// Calculate the consensus threshold, the minimum number of votes for any consensus action to be valid,
    /// for a given number of replicas.
    #[instrument(level = "trace", ret)]
    pub fn threshold(&self) -> usize {
        misc::consensus_threshold(self.validator_set.len())
    }

    /// Calculate the maximum number of faulty replicas, for a given number of replicas.
    #[instrument(level = "trace", ret)]
    pub fn faulty_replicas(&self) -> usize {
        misc::faulty_replicas(self.validator_set.len())
    }
}
