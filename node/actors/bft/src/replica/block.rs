use super::StateMachine;
use crate::{inner::ConsensusInner, io::OutputMessage};
use tracing::{info, instrument};
use zksync_consensus_roles::validator;

impl StateMachine {
    /// Tries to build a finalized block from the given CommitQC. We simply search our
    /// block proposal cache for the matching block, and if we find it we build the block.
    /// If this method succeeds, it sends the finalized block to the executor.
    #[instrument(level = "trace", ret)]
    pub(crate) fn build_block(
        &mut self,
        consensus: &ConsensusInner,
        commit_qc: &validator::CommitQC,
    ) {
        // TODO(gprusak): for availability of finalized blocks,
        //                replicas should be able to broadcast highest quorums without
        //                the corresponding block (same goes for synchronization).
        let Some(cache) = self
            .block_proposal_cache
            .get(&commit_qc.message.proposal.number)
        else {
            return;
        };
        let Some(payload) = cache.get(&commit_qc.message.proposal.payload) else {
            return;
        };
        let block = validator::FinalBlock {
            header: commit_qc.message.proposal,
            payload: payload.clone(),
            justification: commit_qc.clone(),
        };

        info!(
            "Finalized a block!\nFinal block: {:#?}",
            block.header.hash()
        );

        consensus.pipe.send(OutputMessage::FinalizedBlock(block));
    }
}
