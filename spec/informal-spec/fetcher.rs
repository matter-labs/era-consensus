// Asynchronous block fetcher.
// Fetches all the committed blocks that it doesn't have locally from the peers.

fn on_start(replica_state: &ReplicaState) {
    loop {
        let next_block_number = replica_state.committed_blocks.len();
        let Some(commit_qc_ = replica_state.high_commit_qc else { continue };
        if commit_qc.vote.block_number < next_block_number { continue }
        // this function queries other replicas whether they have a CommittedBlock
        // with the given number, and fetches it from them. The fetched block and
        // its commit_qc are verified locally.
        let block : CommittedBlock = self.fetch_block_from_peers(next_block_number);
        self.committed_blocks.push(block);
    }
}
