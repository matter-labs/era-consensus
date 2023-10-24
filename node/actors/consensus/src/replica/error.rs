use crate::validator;
use roles::validator::BlockHeaderHash;
use thiserror::Error;

#[derive(Error, Debug)]
#[allow(clippy::missing_docs_in_private_items)]
pub(crate) enum Error {
    #[error("received leader commit message with invalid leader (correct leader: {correct_leader:?}, received leader: {received_leader:?})]")]
    LeaderCommitInvalidLeader {
        correct_leader: validator::PublicKey,
        received_leader: validator::PublicKey,
    },
    #[error("received leader prepare message with invalid leader (correct leader: {correct_leader:?}, received leader: {received_leader:?}])")]
    LeaderPrepareInvalidLeader {
        correct_leader: validator::PublicKey,
        received_leader: validator::PublicKey,
    },
    #[error("received leader commit message for a past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    LeaderCommitOld {
        current_view: validator::ViewNumber,
        current_phase: validator::Phase,
    },
    #[error("received leader commit message for a past view/phase (current view: {current_view:?}, current phase: {current_phase:?})")]
    LeaderPrepareOld {
        current_view: validator::ViewNumber,
        current_phase: validator::Phase,
    },
    #[error("received leader commit message with invalid signature")]
    LeaderCommitInvalidSignature(#[source] crypto::bls12_381::Error),
    #[error("received leader prepare message with invalid signature")]
    LeaderPrepareInvalidSignature(#[source] crypto::bls12_381::Error),
    #[error("received leader commit message with invalid justification")]
    LeaderCommitInvalidJustification(#[source] anyhow::Error),
    #[error("received leader prepare message with empty map in the justification")]
    LeaderPrepareJustificationWithEmptyMap,
    #[error("received leader prepare message with invalid PrepareQC")]
    LeaderPrepareInvalidPrepareQC(#[source] anyhow::Error),
    #[error("received leader prepare message with invalid high QC")]
    LeaderPrepareInvalidHighQC(#[source] anyhow::Error),
    #[error("received leader prepare message with high QC of a future view (high QC view: {high_qc_view:?}, current view: {current_view:?}")]
    LeaderPrepareHighQCOfFutureView {
        high_qc_view: validator::ViewNumber,
        current_view: validator::ViewNumber,
    },
    #[error("received leader prepare message with new block proposal when the previous proposal was not finalized")]
    LeaderPrepareProposalWhenPreviousNotFinalized,
    #[error("received leader prepare message with new block proposal with invalid parent hash (correct parent hash: {correct_parent_hash:#?}, received parent hash: {received_parent_hash:#?}, block: {block:?})")]
    LeaderPrepareProposalInvalidParentHash {
        correct_parent_hash: BlockHeaderHash,
        received_parent_hash: BlockHeaderHash,
        block: validator::BlockHeader,
    },
    #[error("received leader prepare message with block proposal with non-sequential number (correct proposal number: {correct_number}, received proposal number: {received_number}, block: {block:?})")]
    LeaderPrepareProposalNonSequentialNumber {
        correct_number: u64,
        received_number: u64,
        block: validator::BlockHeader,
    },
    #[error("received leader prepare message with block proposal with an oversized payload (payload size: {payload_size}, block: {block:?}")]
    LeaderPrepareProposalOversizedPayload {
        payload_size: usize,
        block: validator::BlockHeader,
    },
    #[error("received leader prepare message with block re-proposal when the previous proposal was finalized")]
    LeaderPrepareReproposalWhenFinalized,
    #[error("received leader prepare message with block re-proposal of invalid block")]
    LeaderPrepareReproposalInvalidBlock,
}
