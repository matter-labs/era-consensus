//! Replica

// This is the state machine that moves the consensus forward.
struct ReplicaState {
    // The view this replica is currently in.
    view: ViewNumber,
    // The phase of the replica. Determines if we already voted for a proposal
    // this view.
    phase: Phase,
    // The commit vote with the highest view that this replica has signed, if any.
    high_vote: Option<CommitVote>,
    // The commit quorum certificate with the highest view that this replica
    // has observed, if any.
    high_commit_qc: Option<CommitQC>,
    // The timeout quorum certificate with the highest view that this replica
    // has observed, if any.
    high_timeout_qc: Option<TimeoutQC>,
    // A list of every replica (including us) and their respective weights.
    committee: Map<ValidatorPubKey, Weight>,
    // Our public key. It also acts as our identifier.
    pk: ValidatorPubKey,
    // A cache of not-yet-committed proposals.
    // Collects all received proposals sent by the proposers.
    // In real implementation this cache would be actively pruned.
    cached_proposals: Map<(BlockNumber,BlockHash),Block>,
    // A history of committed blocks.
    // invariant: committed_blocks[i].commit_qc.vote.block_number == i
    committed_blocks: Vec<CommittedBlock>,
}

enum Phase {
    Prepare,
    Commit,
    Timeout
}


impl ReplicaState {
    // This is called when the replica starts. At the beginning of the consensus.
    // It is a loop that takes incoming messages and calls the corresponding
    // method for each message.
    fn on_start(&mut self) {
        // Imagine there's a timer util that just has two states (finished or not) and
        // counts down from some given duration. For example, it counts down from 1s.
        // If that time elapses, the timer will change state to finished.
        // If it is reset before that, it starts counting down again from 1s.
        let timer = Timer::new(duration);
        
        // Get the current view.
        let mut cur_view = self.view;
        
        loop {
            // If the view has increased before the timeout, we reset the timer.
            if cur_view < self.view {
                cur_view = self.view;
                timer.reset();
            }
            
            // If the timer has finished, we send a timeout vote.
            // If this is the first view, we immediately timeout. This will force the replicas
            // to synchronize right at the beginning and will provide a justification for the
            // proposer at view 1.
            // If we have already timed out, we don't need to send another timeout vote.
            if (timer.is_finished() || cur_view == 0) && self.phase != Phase::Timeout {
                let vote = TimeoutVote::new(self.view,
                    self.high_vote,
                    self.high_commit_qc);
                    
                // Update our state so that we can no longer vote commit in this view.
                self.phase = Phase::Timeout;
                    
                // Send the vote to all replicas (including ourselves).
                self.send(vote);
            }
                
            // Try to get a message from the message queue and process it. We don't
            // detail the message queue structure since it's boilerplate.
            if let Some(message) = message_queue.pop() {
                match message {
                    Proposal(msg) => {
                        self.on_proposal(msg);
                    }
                    Commit(msg) => {
                        self.on_commit(msg);
                    }
                    Timeout(msg) => {
                        self.on_timeout(msg);
                    }
                    NewView(msg) => {
                        self.on_new_view(msg);
                    }
                }
            }
        }
    } 
        
    fn on_proposal(&mut self, proposal: Proposal) {
        // We only allow proposals for the current view if we have not voted in
        // it yet.
        assert!((proposal.view() == self.view && self.phase == Prepare) || proposal.view() > self.view);
        
        // We ignore proposals from the wrong leader.
        assert!(proposal.leader() == leader(proposal.view()));
        
        // Check that the proposal is valid.
        assert!(proposal.verify());
        
        // Get the implied block number and hash (if any).
        let (block_number, opt_block_hash) = proposal.justification.get_implied_block();
        
        // Vote only if you have collected all committed blocks so far.
        assert!(block_number == self.committed_blocks.last().map_or(0,|b|b.commit_qc.vote.block_number+1));
        
        // Check if this is a reproposal or not, and do the necessary checks.
        // As a side result, get the correct block hash.
        let block_hash = match opt_block_hash {
            Some(hash) => {
                // This is a reproposal.
                // We let the leader repropose blocks without sending them in the proposal
                // (it sends only the block number + block hash). That allows a leader to
                // repropose a block without having it stored. Sending reproposals without
                // a payload is an optimization that allows us to not wait for a leader that
                // has the previous proposal stored (which can take 4f views), and to somewhat
                // speed up reproposals by skipping block broadcast.
                // This only saves time because we have a gossip network running in parallel,
                // and any time a replica is able to create a finalized block (by possessing
                // both the block and the commit QC) it broadcasts the finalized block (this
                // was meant to propagate the block to full nodes, but of course validators
                // will end up receiving it as well).
                
                // We check that the leader didn't send a payload with the reproposal.
                // This isn't technically needed for the consensus to work (it will remain
                // safe and live), but it's a good practice to avoid unnecessary data in
                // blockchain.
                // This unnecessary payload would also effectively be a source of free
                // data availability, which the leaders would be incentivized to abuse.
                assert!(proposal.block.is_none());
                
                hash
            }
            None => {
                // This is a new proposal, so we need to verify it (i.e. execute it).
                assert!(proposal.block.is_some());
                let block = proposal.block.unwrap();
                // To verify the block, replica just tries to apply it to the current
                // state. Current state is the result of applying all the committed blocks until now.
                assert!(self.verify_block(block_number, block));
                // We cache the new proposals, waiting for them to be committed.
                self.cached_proposals.insert((block_number,proposal.block.hash()),block);
                block.hash()
            }
        };
        
        // Update the state.
        let vote = CommitVote::new(proposal.view(), block_number, block_hash);
        
        self.view = proposal.view();
        self.phase = Phase::Commit;
        self.high_vote = Some(vote);
        match proposal.justification {
            Commit(qc) => self.process_commit_qc(Some(qc)),
            Timeout(qc) => {
                self.process_commit_qc(qc.high_commit_qc);
                self.high_timeout_qc = max(Some(qc), self.high_timeout_qc);
            }
        };
        
        // Send the commit vote to all replicas (including ourselves).
        self.send(vote);
    }
    
    // Processes a (already verified) commit_qc received from the network
    // as part of some message. It bumps the local high_commit_qc and if
    // we have the proposal corresponding to this qc, we append it to the committed_blocks.
    fn process_commit_qc(&mut self, qc_opt: Option<CommitQC>) {
        if let Some(qc) = qc_opt {
            self.high_commit_qc = max(Some(qc), self.high_commit_qc);
            let Some(block) = self.cached_proposals.get((qc.vote.block_number,qc.vote.block_hash)) else { return };
            if self.committed_blocks.len()==qc.vote.block_number {
                self.committed_blocks.push(CommittedBlock{block,commit_qc:qc});
            }
        }
    }
    
    fn on_commit(&mut self, sig_vote: SignedCommitVote) {
        // If the vote isn't current, just ignore it.
        assert!(sig_vote.view() >= self.view)
        
        // Check that the signed vote is valid.
        assert!(sig_vote.verify());
        
        // Store the vote. We will never store duplicate (same view and sender) votes.
        // If we already have this vote, we exit early.
        assert!(self.store(sig_vote).is_ok());
        
        // Check if we now have a commit QC for this view.
        if let Some(qc) = self.get_commit_qc(sig_vote.view()) {
            self.process_commit_qc(Some(qc));
            self.start_new_view(sig_vote.view() + 1);
        }
    }
    
    fn on_timeout(&mut self, sig_vote: SignedTimeoutVote) {
        // If the vote isn't current, just ignore it.
        assert!(sig_vote.view() >= self.view)
        
        // Check that the signed vote is valid.
        assert!(sig_vote.verify());
        
        // Store the vote. We will never store duplicate (same view and sender) votes.
        // If we already have this vote, we exit early.
        assert!(self.store(sig_vote).is_ok());
        
        // Check if we now have a timeout QC for this view.
        if let Some(qc) = self.get_timeout_qc(sig_vote.view()) {
            self.process_commit_qc(qc.high_commit_qc);
            self.high_timeout_qc = max(Some(qc), self.high_timeout_qc);
            self.start_new_view(sig_vote.view() + 1);
        }
    }
    
    fn on_new_view(&mut self, new_view: NewView) {
        // If the message isn't current, just ignore it.
        assert!(new_view.view() >= self.view)
        
        // Check that the new view message is valid.
        assert!(new_view.verify());
        
        // Update our state.
        match new_view.justification {
            Commit(qc) => self.process_commit_qc(Some(qc)),
            Timeout(qc) => {
                self.process_commit_qc(qc.high_commit_qc);
                self.high_timeout_qc = max(Some(qc), self.high_timeout_qc);
            }
        };
        
        if new_view.view() > self.view {
            self.start_new_view(new_view.view());
        }
    }
    
    fn start_new_view(&mut self, view: ViewNumber) {
        self.view = view;
        self.phase = Phase::Prepare;
        
        // Send a new view message to the other replicas, for synchronization.
        let new_view = NewView::new(self.create_justification());
        
        self.send(new_view);
    }

    fn create_justification(&self) {
        // We need some QC in order to be able to create a justification.
        assert!(self.high_commit_qc.is_some() || self.high_timeout_qc.is_some());

        if self.high_commit_qc.map(|x| x.view()) >= self.high_timeout_qc.map(|x| x.view()) {
            Justification::Commit(self.high_commit_qc.unwrap())
        } else {
            Justification::Timeout(self.high_timeout_qc.unwrap())
        }
    }
}
