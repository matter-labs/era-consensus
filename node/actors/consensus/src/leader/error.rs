use roles::validator;
use thiserror::Error;

#[derive(Error, Debug)]
#[allow(clippy::missing_docs_in_private_items)]
pub(crate) enum Error {
    #[error("received replica commit message for a missing proposal")]
    ReplicaCommitMissingProposal,
    #[error("received replica commit message for a past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    ReplicaCommitOld {
        current_view: validator::ViewNumber,
        current_phase: validator::Phase,
    },
    #[error("received replica prepare message for a past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    ReplicaPrepareOld {
        current_view: validator::ViewNumber,
        current_phase: validator::Phase,
    },
    #[error("received replica commit message for a view when we are not a leader")]
    ReplicaCommitWhenNotLeaderInView,
    #[error("received replica prepare message for a view when we are not a leader")]
    ReplicaPrepareWhenNotLeaderInView,
    #[error("received replica commit message that already exists (existing message: {existing_message:?}")]
    ReplicaCommitExists { existing_message: String },
    #[error("received replica prepare message that already exists (existing message: {existing_message:?}")]
    ReplicaPrepareExists { existing_message: String },
    #[error("received replica commit message while number of received messages is below threshold. waiting for more (received: {num_messages:?}, threshold: {threshold:?}")]
    ReplicaCommitNumReceivedBelowThreshold {
        num_messages: usize,
        threshold: usize,
    },
    #[error("received replica prepare message while number of received messages below threshold. waiting for more (received: {num_messages:?}, threshold: {threshold:?}")]
    ReplicaPrepareNumReceivedBelowThreshold {
        num_messages: usize,
        threshold: usize,
    },
    #[error("received replica prepare message with high QC of a future view (high QC view: {high_qc_view:?}, current view: {current_view:?}")]
    ReplicaPrepareHighQCOfFutureView {
        high_qc_view: validator::ViewNumber,
        current_view: validator::ViewNumber,
    },
    #[error("received replica commit message with invalid signature")]
    ReplicaCommitInvalidSignature(#[source] crypto::bls12_381::Error),
    #[error("received replica prepare message with invalid signature")]
    ReplicaPrepareInvalidSignature(#[source] crypto::bls12_381::Error),
    #[error("received replica prepare message with invalid high QC")]
    ReplicaPrepareInvalidHighQC(#[source] anyhow::Error),
}

/// Needed due to source errors.
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}
