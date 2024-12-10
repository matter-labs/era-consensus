// The sum of all validator weights.
const TOTAL_WEIGHT = committee.sum_weights();
// The maximum weight of faulty replicas.
// We want 5*FAULTY_WEIGHT + 1 = TOTAL_WEIGHT
const FAULTY_WEIGHT = (TOTAL_WEIGHT - 1) / 5;
// The weight threshold needed to form a quorum certificate.
const QUORUM_WEIGHT = TOTAL_WEIGHT - FAULTY_WEIGHT;
// The weight threshold needed to trigger a reproposal.
const SUBQUORUM_WEIGHT = TOTAL_WEIGHT - 3 * FAULTY_WEIGHT;

/// A block with a matching valid certificate.
/// invariants: hash(block) == commit_qc.vote.block_hash
struct CommittedBlock {
    block: Block,
    commit_qc: CommitQC,
}

// Messages

struct Proposal {
    // Block that the leader is proposing, if this is a new proposal.
    block: Option<Block>,
    // What attests to the validity of this message.
    justification: Justification,
    // Signature of the sender.
    sig: Signature,
}

impl Proposal {
    fn view(self) -> ViewNumber {
        self.justification.view()
    }

    fn verify(self) -> bool {
        self.verify_sig() && self.justification.verify()
    }
}

enum Justification {
    // This proposal is being proposed after a view where we finalized a block.
    // A commit QC is just a collection of commit votes (with at least
    // QUORUM_WEIGHT) for the previous view. Note that the commit votes MUST
    // be identical.
    Commit(CommitQC),
    // This proposal is being proposed after a view where we timed out.
    // A timeout QC is just a collection of timeout votes (with at least
    // QUORUM_WEIGHT) for the previous view. Unlike with the Commit QC,
    // timeout votes don't need to be identical.
    // The first proposal, for view 0, will always be a timeout.
    Timeout(TimeoutQC),
}

impl Justification {
    fn view(self) -> ViewNumber {
        match self {
            Commit(qc) => qc.view() + 1,
            Timeout(qc) => qc.view() + 1,
        }
    }

    fn verify(self) -> bool {
        match self {
            Commit(qc) => qc.verify(),
            Timeout(qc) => qc.verify(),
        }
    }

    // This returns the block number that is implied by this justification.
    // If the justification requires a block reproposal, it also returns
    // the hash of the block that must be reproposed.
    fn get_implied_block(self) -> (BlockNumber, Option<Hash>) {
        match self {
            Commit(qc) => {
                // The previous proposal was finalized, so we can propose a new block.
                (qc.vote.block_number + 1, None)
            }
            Timeout(qc) => {
                // Get the high vote of the timeout QC, if it exists. We check if there are
                // timeout votes with at least an added weight of SUBQUORUM_WEIGHT,
                // that have a high vote field for the same block. A QC can have
                // 0, 1 or 2 such blocks.
                // If there's only 1 such block, then we say the QC has a high vote.
                // If there are 0 or 2 such blocks, we say the QC has no high vote.
                let high_vote: Option<CommitVote> = qc.high_vote();

                // Get the high commit QC of the timeout QC. We compare the high QC field of
                // all timeout votes in the QC, and get the highest one, if it exists.
                let high_qc: Option<CommitQC> = qc.high_commit_qc;

                if high_vote.is_some()
                    && (high_qc.is_none() || high_vote.block_number > high_qc.vote.block_number)
                {
                    // There was some proposal last view that might have been finalized.
                    // We need to repropose it.
                    (high_vote.block_number, Some(high_vote.block_hash))
                } else {
                    // Either the previous proposal was finalized or we know for certain
                    // that it couldn't have been finalized. Either way, we can propose
                    // a new block.
                    let block_number = match high_qc {
                        Some(qc) => qc.vote.block_number + 1,
                        None => 0,
                    };

                    (block_number, None)
                }
            }
        }
    }
}

struct CommitVote {
    // The current view.
    view: ViewNumber,
    // The number of the block that the replica is committing to.
    block_number: BlockNumber,
    // The hash of the block that the replica is committing to.
    block_hash: BlockHash,
}

struct SignedCommitVote {
    // The commit.
    vote: CommitVote,
    // Signature of the sender.
    sig: Signature,
}

impl SignedCommitVote {
    fn view(self) -> ViewNumber {
        self.vote.view()
    }

    fn verify(self) -> bool {
        self.verify_sig()
    }
}

// If we have identical commit votes with at least QUORUM_WEIGHT
// from different replicas, we can create a commit quorum certificate locally.
struct CommitQC {
    // The commit.
    vote: CommitVote,
    // The aggregate signature of the replicas. We ignore the details here.
    // Can be something as simple as a vector of signatures.
    agg_sig: AggregateSignature,
}

impl CommitQC {
    fn view(self) -> ViewNumber {
        self.vote.view()
    }

    fn verify(self) -> bool {
        // In here we need to not only check the signature, but also that
        // it has enough weight beyond it.
        self.verify_agg_sig(QUORUM_WEIGHT)
    }
}

struct TimeoutVote {
    // The current view.
    view: ViewNumber,
    // The commit vote with the highest view that this replica has signed, if any.
    high_vote: Option<CommitVote>,
    // The view number of the highest commit quorum certificate that this replica
    // has observed, if any.
    high_commit_qc_view: Option<ViewNumber>,
}

struct SignedTimeoutVote {
    // The timeout.
    vote: TimeoutVote,
    // Signature of the sender. This signature is ONLY over the vote field.
    sig: Signature,
    // The commit quorum certificate with the highest view that this replica
    // has observed, if any. It MUST match `high_commit_qc_view` in `vote`.
    high_commit_qc: Option<CommitQC>,
}

impl SignedTimeoutVote {
    fn view(self) -> ViewNumber {
        self.vote.view()
    }

    fn verify(self) -> bool {
        // If we wish, there are three invariants that are easy to check but don't need to be strictly enforced for correctness:
        // 1. self.view() >= self.high_vote.view()
        // 2. self.high_vote.view() >= self.high_commit_qc_view
        // 3. self.view() > self.high_commit_qc_view
        self.vote.high_commit_qc_view == self.high_commit_qc.map(|x| x.view()) && self.verify_sig()
          && self.high_commit_qc.map(|qc| qc.verify())
    }
}

// If we have timeout votes with at least QUORUM_WEIGHT for the same
// view from different replicas, we can create a timeout quorum certificate
// locally. Contrary to the commit QC, this QC aggregates different messages.
struct TimeoutQC {
    // A collection of the replica timeout votes, indexed by the corresponding
    // validator public key.
    // There are better data structures for this. This is just for clarity.
    votes: Map<ValidatorPubKey, TimeoutVote>,
    // The aggregate signature of the replicas. We ignore the details here.
    // Can be something as simple as a vector of signatures.
    agg_sig: AggregateSignature,
    // The commit quorum certificate with the highest view that all replicas in this
    // QC have observed, if any. It MUST match the highest `high_commit_qc_view` in `votes`.
    high_commit_qc: Option<CommitQC>,
}

impl TimeoutQC {
    fn view(self) -> ViewNumber {
        self.votes[0].view()
    }

    fn verify(self) -> bool {
        // Check that all votes have the same view and get the highest commit QC view of the timeout QC.
        let high_qc_view = None;

        for (_, vote) in self.votes {
            if vote.view != self.view() {
                return false;
            }
            high_qc_view = max(high_qc_view, vote.high_commit_qc_view);
        }

        // In here we need to not only check the signature, but also that
        // it has enough weight beyond it.
        if !self.verify_agg_sig(QUORUM_WEIGHT) {
            return false;
        }

        // We check that the high commit QC view matches the high commit QC that we have, and we verify the QC.
        match self.high_commit_qc {
            Some(high_qc) => high_qc_view == Some(high_qc.view()) && high_qc.verify();
            None => high_qc_view.is_none();
        }
    }

    fn high_vote(self) -> Option<CommitVote> {
        let map: Map<CommitVote, Weight> = Map::new();

        for (pk, vote) in votes {
            if let Some(commit_vote) = vote.high_vote {
                if map.exists(commit_vote) {
                    map.get_value(commit_vote).add(get_weight(pk));
                } else {
                    map.insert(commit_vote, get_weight(pk));
                }
            }
        }

        let subquorums = Vec::new();

        for (vote, weight) in map {
            if weight >= SUBQUORUM_WEIGHT {
                subquorums.push(vote);
            }
        }

        // We only consider there's a high vote if exactly one subquorum has been found.
        // If there are 0 or 2+ subquorums, we consider there's no high vote.
        //
        // For the case of 0 subquorums, we consider there's no high vote because
        // if the previous proposal received n-f votes (thus being finalized) then the
        // intersection of those n-f nodes with the n-f nodes that voted in this timeout
        // would have at least n-3f *honest* nodes that committed to the previous proposal
        // (f honest nodes that didn't commit in the previous round, f honest nodes that didn't
        // timeout this round, and f dishonest nodes that did timeout this round), and thus we
        // would have a subquorum.
        //
        // For the case of 2 subquorums, we consider there's no high vote because if we have
        // n-3f votes for one block and n-3f votes for another block, for a total of 2n-6f votes,
        // then there not enough remaining votes for any of the blocks to be finalized. Note that
        // we have a total of n+f votes (one vote for each node, plus an extra one from each of the
        // dishonest nodes since they can vote twice). So we have n+f - (2n-6f) = 7f-n, since n>=5f+1
        // we have 7f-5f-1 = 2f-1, so there's at most 2f-1 votes remaining, which is not enough to
        // finalize any block.
        // For >2 subquorums, the argument is similar.
        if subquorums.len() == 1 {
            Some(subquorums[0])
        } else {
            None
        }
    }
}

struct NewView {
    // What attests to the validity of this view change.
    justification: Justification,
    // Signature of the sender.
    sig: Signature,
}

impl NewView {
    fn view(self) -> ViewNumber {
        self.justification.view()
    }

    fn verify(self) -> bool {
        self.verify_sig() && self.justification.verify()
    }
}
