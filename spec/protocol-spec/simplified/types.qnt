// -*- mode: Bluespec; -*-

/**
 * Types for simplified ChonkyBFT.
 */
module types {
    import option.* from "../option"

    // For specification purposes, a block is just an indivisible string.
    // We can simply use name such as "v0", "v1", "v2". What matters here
    // is that these names are unique.
    type Block = str

    // For specification purposes, we represent a hash of a string `s` as
    // the string `s`. This representation is collision-free,
    // and we interpret it as opaque.
    type BlockHash = str

    // Get the "hash" of a string
    pure def hash(b: str): BlockHash = b

    // View number
    type ViewNumber = int

    // Block number
    type BlockNumber = int

    // Weight
    type Weight = int

    // A replica id is a unique replica identifier, e.g., its hardware address.
    // For specification purporses, these are just indivisible strings
    // such as "n0", "n1", "n2", "n3". What matters to us is that these names are unique.
    type ReplicaId = str

    // Each replica id has a private/public key pair assigned to it.
    // For testing purposes and for modeling byzantine behavior,
    // several replica ids may have the private/key pair.
    type ReplicaKey = str

    // Similar to ReplicaId, we model the signature as the replica name.
    type Signature = str

    // For specification purposes, an aggregate signature is simply a set
    // of signatures.
    type AggregateSignature = Set[Signature]

    // An identifier assigned to TimeoutQC or CommitQC.
    type QCName = int

    /// A block with a matching valid certificate.
    /// invariants: hash(block) == commit_qc.vote.block_hash
    type CommittedBlock = {
        block: Block,
        commit_qc: CommitQC,
    }

    type ReplicaState = {
        // The current view number.
        view: ViewNumber,
        // The phase of the replica. Determines if we already voted for a proposal in this view.
        // Phase::Prepare | Phase::Commit | Phase::Timeout
        phase: Phase,
        // The commit vote with the highest view that this replica has signed, if any.
        high_vote: Option[CommitVote],
        // The commit quorum certificate with the highest view that this replica
        // has observed, if any.
        high_commit_qc: Option[CommitQC],
        // The timeout quorum certificate with the highest view that this replica
        // has observed, if any.
        high_timeout_qc: Option[TimeoutQC],
        // A cache of not-yet-committed proposals.
        // Collects all received proposals sent by the proposers.
        // In real implementation this cache would be actively pruned.
        // In this Quint specification, blocks and their hashes are identical,
        // so we store the set Set[(BlockNumber, BlockHash)] instead of a map
        // from (BlockNumber, BlockHash) to Block.
        cached_proposals: Set[(BlockNumber, BlockHash)],
        // A history of committed blocks.
        // invariant: committed_blocks[i].commit_qc.vote.block_number == i
        committed_blocks: List[CommittedBlock],
    }

    type Phase = PhasePrepare | PhaseCommit | PhaseTimeout

    type Proposal = {
        // Block that the leader is proposing, if this is a new proposal.
        block: Option[Block],
        // What attests to the validity of this proposal.
        justification: Justification,
        // Signature of the sender.
        sig: Signature,
        // The block number of the proposed block, if Some; otherwise 0.
        // We assume that the implementation can extract the block number from the block.
        ghost_block_number: BlockNumber,
    }

    type Justification =
            // This proposal is being proposed after a view where we finalized a block.
            // A commit QC is just a collection of commit votes (with at least
            // QUORUM_WEIGHT) for the previous view. Note that the commit votes MUST
            // be identical.
            Commit(CommitQC)
            // This proposal is being proposed after a view where we timed out.
            // A timeout QC is just a collection of timeout votes (with at least
            // QUORUM_WEIGHT) for the previous view. Unlike with the Commit QC,
            // timeout votes don't need to be identical.
        |   Timeout(TimeoutQC)


    type CommitVote = {
        // The current view.
        view: ViewNumber,
        // The number of the block that the replica is committing to.
        block_number: BlockNumber,
        // The hash of the block that the replica is committing to.
        block_hash: BlockHash,
    }

    type SignedCommitVote = {
        // The commit.
        vote: CommitVote,
        // Signature of the sender.
        sig: Signature,
    }

    pure val COMMIT_VOTE_NONE: Option[CommitVote] = None

    // If we have 4f+1 identical commit votes from different replicas, we can
    // create a commit quorum certificate locally.
    type CommitQC = {
        // The commit.
        vote: CommitVote,
        // The aggregate signature of the replicas. We ignore the details here.
        // Can be something as simple as a vector of signatures.
        agg_sig: AggregateSignature,
    }

    pure val HIGH_COMMIT_QC_NONE: Option[CommitQC] = None

    // Timeout votes are sent from replicas to replicas.
    type TimeoutVote = {
        // The current view.
        view: ViewNumber,
        // The commit vote with the highest view that this replica has signed.
        high_vote: Option[CommitVote],
        // The view number of the highest commit quorum certificate that this replica
        // has observed, if any.
        high_commit_qc_view: Option[ViewNumber],
    }

    pure val VIEW_NUMBER_NONE: Option[ViewNumber] = None

    type SignedTimeoutVote = {
        // The timeout.
        vote: TimeoutVote,
        // Signature of the sender. This signature is ONLY over the vote field.
        sig: Signature,
        // The commit quorum certificate with the highest view that this replica
        // has observed, if any. It MUST match `high_commit_qc_view` in `vote`.
        high_commit_qc: Option[CommitQC],
    }

    // If we have timeout votes with at least QUORUM_WEIGHT for the same
    // view from different replicas, we can create a timeout quorum certificate
    // locally. Contrary to the commit QC, this QC aggregates different messages.
    type TimeoutQC = {
        // A collection of the replica timeout votes, indexed by the corresponding
        // validator public key.
        // We have to make sure that a vote from each replica appears here at most once.
        votes: ReplicaKey -> TimeoutVote,
        // The aggregate signature of the replicas. We ignore the details here.
        // Can be something as simple as a vector of signatures.
        agg_sig: AggregateSignature,
        // The commit quorum certificate with the highest view that all replicas in this
        // QC have observed, if any. It MUST match the highest `high_commit_qc_view` in `votes`.
        high_commit_qc: Option[CommitQC],

        // current view computed from the votes (not present in the original specification).
        ghost_view: ViewNumber,
    }

    pure val HIGH_TIMEOUT_QC_NONE: Option[TimeoutQC] = None

    // New view messages are sent from replicas to replicas.
    // These are used to synchronize replicas.
    type NewView = {
        // What attests to the validity of this view change.
        justification: Justification,
        // Signature of the sender.
        sig: Signature,
    }

    type StepKind =
          InitStep | ProposerStep(ReplicaId) | OnProposalStep(ReplicaId) | OnCommitStep(ReplicaId)
        | OnTimeoutStep(ReplicaId) | OnTimerIsFinishedStep(ReplicaId)
        | OnNewViewStep({ id: ReplicaId, view: ViewNumber }) | FaultyStep | FetchStep(ReplicaId) | AnyStep
}