use super::StateMachine;
use crate::{inner::ConsensusInner, io::OutputMessage};
use roles::validator;
use tracing::{info, instrument};

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
        if let Some(block) = self
            .block_proposal_cache
            .get(&commit_qc.message.proposal_block_number)
            .and_then(|m| m.get(&commit_qc.message.proposal_block_hash))
        {
            let final_block = validator::FinalBlock {
                block: block.clone(),
                justification: commit_qc.clone(),
            };

            info!(
                "Finalized a block!\nFinal block: {:#?}",
                final_block.block.hash()
            );

            consensus
                .pipe
                .send(OutputMessage::FinalizedBlock(final_block));
        }
    }
}
