//! The inner data of the consensus state machine. This is shared between the different roles.

use crate::{
    io::{InputMessage, OutputMessage},
    misc,
};
use zksync_concurrency::{ctx::channel};
use tracing::instrument;
use zksync_consensus_roles::validator;
use zksync_consensus_utils::pipe::ActorPipe;

/// The ConsensusInner struct, it contains data to be shared with the state machines. This is never supposed
/// to be modified, except by the Consensus struct.
#[derive(Debug)]
pub(crate) struct ConsensusInner {
    /// The communication pipe. This is used to receive inputs and send outputs.
    pub(crate) pipe: channel::UnboundedSender<OutputMessage>,
    /// The validator's secret key.
    pub(crate) secret_key: validator::SecretKey,
    /// A vector of public keys for all the validators in the network.
    pub(crate) validator_set: validator::ValidatorSet,
}

impl ConsensusInner {
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
        let num_validators = self.validator_set.len();

        misc::consensus_threshold(num_validators)
    }

    /// Calculate the maximum number of faulty replicas, for a given number of replicas.
    #[instrument(level = "trace", ret)]
    pub fn faulty_replicas(&self) -> usize {
        let num_validators = self.validator_set.len();

        misc::faulty_replicas(num_validators)
    }
}
