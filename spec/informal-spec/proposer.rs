// Proposer

// This is called when the proposer starts. At the beginning of the consensus.
fn on_start(replica_state: &ReplicaState) {
    // The proposer has visibility into the replica state. But CANNOT write to it.
    let mut cur_view = replica_state.view;

    loop {
        // Check if we are in a new view and we are the leader.
        // Note that by definition, it's impossible to be the leader for view 0.
        // We always let view 0 timeout.
        if cur_view < replica_state.view && replica_state.pk == leader(replica_state.view) {
            cur_view = replica_state.view;

            // Get the justification for this view. If we have both a commit QC
            // and a timeout QC for this view (highly unlikely), we should prefer
            // to use the commit QC.
            let justification = replica_state.create_justification();

            assert!(justification.view() == cur_view);

            // Get the block number and check if this must be a reproposal.
            let (block_number, opt_block_hash) = justification.get_implied_block();

            // Propose only if you have collected all committed blocks so far.
            assert!(
                block_number
                    == self
                        .committed_blocks
                        .last()
                        .map_or(0, |b| b.commit_qc.vote.block_number + 1)
            );

            // Now we create the block.
            let block = if opt_block_hash.is_some() {
                // There was some proposal last view that at least SUBQUORUM_WEIGHT replicas
                // voted for and could have been finalized. We need to repropose it.
                None
            } else {
                // The previous proposal was finalized, so we can propose a new block.
                // If we don't have the previous block, this call will fail. This is
                // fine as we can just not propose anything and let our turn end.
                Some(self.create_proposal(block_number))
            };

            // Create the proposal message and send it to all replicas
            let proposal = Proposal::new(block, justification);

            self.send(proposal);
        }
    }
}
