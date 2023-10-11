use thiserror::Error;
use roles::validator;

#[derive(Error, Debug)]
#[allow(clippy::missing_docs_in_private_items)]
pub(crate) enum ReplicaMessageError {
    #[error("commit for a missing proposal")]
    CommitMissingProposal,
    #[error("commit for a past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    CommitOld {
        current_view: validator::ViewNumber,
        current_phase: validator::Phase,
    },
    #[error("prepare for a past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    PrepareOld {
        current_view: validator::ViewNumber,
        current_phase: validator::Phase,
    },
    #[error("commit for a view when we are not a leader")]
    CommitWhenNotLeaderInView,
    #[error("prepare for a view when we are not a leader")]
    PrepareWhenNotLeaderInView,
    #[error("commit duplicated (existing message: {existing_message:?}")]
    CommitDuplicated {
        existing_message: String
    },
    #[error("prepare duplicated (existing message: {existing_message:?}")]
    PrepareDuplicated {
        existing_message: String
    },
    #[error("commit while number of received messages below threshold, waiting for more (received: {num_messages:?}, threshold: {threshold:?}")]
    CommitNumReceivedBelowThreshold {
        num_messages: usize,
        threshold: usize,
    },
    #[error("prepare while number of received messages below threshold, waiting for more (received: {num_messages:?}, threshold: {threshold:?}")]
    PrepareNumReceivedBelowThreshold {
        num_messages: usize,
        threshold: usize,
    },
    #[error("prepare with high QC of a future view (high QC view: {high_qc_view:?}, current view: {current_view:?}")]
    PrepareHighQCOfFutureView {
        high_qc_view: validator::ViewNumber,
        current_view: validator::ViewNumber,
    },
    #[error("commit with invalid signature")]
    CommitInvalidSignature {
        inner_err: crypto::bls12_381::Error
    },
    #[error("prepare with invalid signature")]
    PrepareInvalidSignature {
        inner_err: crypto::bls12_381::Error
    },
    #[error("prepare with invalid high QC")]
    PrepareInvalidHighQC {
        inner_err: anyhow::Error
    }
}


/// Needed due to inner errors.
impl PartialEq for ReplicaMessageError {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}