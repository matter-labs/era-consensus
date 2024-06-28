// Scaffolding for proving safety with inductive invariants
//
// Check with:
// quint verify --max-steps=1 --invariant=inv \
//   --apalache-config=experiments/apalache-inductive.json inductive_n6f1b0.qnt
module inductive {
    import types.* from "./types"
    import option.* from "./option"
    
    const IWEIGHTS: ReplicaId -> Weight
    const IREPLICA_KEYS: ReplicaId -> ReplicaKey
    const IN: int
    const IF: int
    const ICORRECT: Set[ReplicaId]
    const IFAULTY: Set[ReplicaId]
    const IVIEWS: Set[ViewNumber]
    const IVALID_BLOCKS: Set[Block]
    const IINVALID_BLOCKS: Set[Block]

    import replica(
        WEIGHTS = IWEIGHTS,
        REPLICA_KEYS = IREPLICA_KEYS,
        N = IN,
        F = IF,
        CORRECT = ICORRECT,
        FAULTY = IFAULTY,
        VIEWS = IVIEWS,
        VALID_BLOCKS = IVALID_BLOCKS,
        INVALID_BLOCKS = IINVALID_BLOCKS
    ) as r from "./replica"

    // to make the state visible to the instances
    val state = {
        replica_state: r::replica_state,
        leader: r::leader,
        replica_view: r::replica_view,
        proposer_view: r::replica_view,
        msgs_signed_commit: r::msgs_signed_commit,
        msgs_signed_timeout: r::msgs_signed_timeout,
    }

    // All possible values of CommitVote under fixed parameters.
    // For |VIEWS| = 4, |ALL_BLOCKS| = 3, we have 48 elements.
    pure val ALL_COMMIT_VOTES: Set[CommitVote] =
        tuples(r::VIEWS, r::VIEWS, r::ALL_BLOCKS).map(((v, bn, bh)) => {
            view: v, block_number: bn, block_hash: bh,
        })

    // All possible values of SignedCommitVote under fixed parameters.
    // For |REPLICAS| = 6, |VIEWS| = 4, |ALL_BLOCKS| = 3, we have 288 elements.
    pure val ALL_SIGNED_COMMIT_VOTES: Set[SignedCommitVote] =
        tuples(r::REPLICAS, r::VIEWS, r::VIEWS, r::ALL_BLOCKS).map(((id, v, bn, bh)) => {
            sig: r::sig_of_id(id),
            vote: { view: v, block_number: bn, block_hash: bh },
        })

    // All possible values of CommitQC under fixed parameters.
    // For |REPLICAS| = 6, |VIEWS| = 4, |ALL_BLOCKS| = 3, we have 18432 elements.
    pure val ALL_COMMIT_QCS: Set[CommitQC] =
        tuples(ALL_COMMIT_VOTES, r::REPLICAS.powerset()).map(((vote, agg_sig)) => {
            vote: vote, agg_sig: agg_sig,
        })

    // All possible values of TimeoutVote under fixed parameters.
    // For |REPLICAS| = 6, |VIEWS| = 4, |ALL_BLOCKS| = 3, we have 2359296 elements.
    pure val ALL_TIMEOUT_VOTES: Set[TimeoutVote] =
        tuples(r::VIEWS, ALL_COMMIT_VOTES, Bool, ALL_COMMIT_VOTES, r::REPLICAS.powerset(), Bool)
            .map(((v, hv, is_some_hv, qc_vote, qc_agg_sig, is_some_qc)) => {
                view: v,
                high_vote: if (is_some_hv) Some(hv) else COMMIT_VOTE_NONE,
                high_commit_qc:
                    if (is_some_qc) {
                        Some({ vote: qc_vote, agg_sig: qc_agg_sig })
                    } else {
                        HIGH_COMMIT_QC_NONE
                    },
            })

    // all possible values of SignedTimeoutVote under fixed parameters
    // For |REPLICAS| = 6, |VIEWS| = 4, |ALL_BLOCKS| = 3, we have 14155776 elements.
    pure val ALL_SIGNED_TIMEOUT_VOTES: Set[SignedTimeoutVote] =
        tuples(r::REPLICAS, r::VIEWS, ALL_COMMIT_VOTES, Bool, ALL_COMMIT_VOTES, r::REPLICAS.powerset(), Bool)
            .map(((id, v, hv, is_some_hv, qc_vote, qc_agg_sig, is_some_qc)) => {
                sig: r::sig_of_id(id),
                vote: {
                    view: v,
                    high_vote: if (is_some_hv) Some(hv) else COMMIT_VOTE_NONE,
                    high_commit_qc:
                        if (is_some_qc) {
                            Some({ vote: qc_vote, agg_sig: qc_agg_sig })
                        } else {
                            HIGH_COMMIT_QC_NONE
                        },
                }
            })

    // All possible values of TimeoutQC under fixed parameters.
    // For |REPLICAS| = 6, |VIEWS| = 4,
    // we have 2359296^(2^6) * (2^6) * 4 = about 10^410 elements.
    pure val ALL_TIMEOUT_QCS: Set[TimeoutQC] =
        pure val ALL_VOTES = r::REPLICAS.powerset().map(voters => voters.setOfMaps(ALL_TIMEOUT_VOTES)).flatten()
        tuples(ALL_VOTES, r::REPLICAS.powerset(), r::VIEWS).map(((votes, agg_sig, ghost_view)) => {
            votes: votes, agg_sig: agg_sig, ghost_view: ghost_view
        })

    pure val COMMIT_VOTE_SAMPLE: CommitVote = {
        view: 0, block_number: 0, block_hash: "val_b0",
    }

    pure val COMMIT_QC_SAMPLE: CommitQC = {
        vote: COMMIT_VOTE_SAMPLE,
        agg_sig: Set("n0"),
    }

    pure val TIMEOUT_VOTE_SAMPLE: TimeoutVote = {
        view: 0,
        high_vote: Some(COMMIT_VOTE_SAMPLE),
        high_commit_qc: Some(COMMIT_QC_SAMPLE),
    }

    pure val JUSTIFICATION_SAMPLE: Justification = {
        Commit({ vote: COMMIT_VOTE_SAMPLE , agg_sig: Set("n0") })
    }

    pure val PROPOSAL_SAMPLE: Proposal = {
        block: Some("val_b0"),
        justification: JUSTIFICATION_SAMPLE,
        sig: "n0",
        ghost_block_number: 0,
    }

    pure val TIMEOUT_QC_SAMPLE: TimeoutQC = {
        votes: Set("n0").mapBy(id => TIMEOUT_VOTE_SAMPLE),
        agg_sig: Set("n0"),
        ghost_view: 0,
    }

    pure val VIEW_SAMPLE: NewView = {
        justification: JUSTIFICATION_SAMPLE,
        sig: "n0",
    }

    pure val COMMITTED_BLOCK_SAMPLE: CommittedBlock = {
        block: "val_b0",
        commit_qc: COMMIT_QC_SAMPLE,
    }

    pure def commit_vote_shape_inv(v: CommitVote): bool = and {
        r::VIEWS.contains(v.view),
        r::VIEWS.contains(v.block_number),
        r::ALL_BLOCKS.contains(v.block_hash),
    }

    pure def signed_commit_vote_shape_inv(sv: SignedCommitVote): bool = and {
        r::REPLICAS.contains(sv.sig),
        commit_vote_shape_inv(sv.vote)
    }

    pure def commit_qc_shape_inv(qc: CommitQC): bool = and {
        commit_vote_shape_inv(qc.vote),
        qc.agg_sig.subseteq(r::REPLICAS),
    }

    pure def timeout_vote_shape_inv(v: TimeoutVote): bool = and {
        r::VIEWS.contains(v.view),
        v.high_vote.option_has(commit_vote_shape_inv),
        v.high_commit_qc.option_has(commit_qc_shape_inv),
    }

    pure def signed_timeout_vote_shape_inv(sv: SignedTimeoutVote): bool = and {
        r::REPLICAS.contains(sv.sig),
        timeout_vote_shape_inv(sv.vote),
    }

    pure def timeout_qc_shape_inv(qc: TimeoutQC): bool = and {
        qc.votes.keys().forall(id => and {
            r::REPLICAS.contains(id),
            timeout_vote_shape_inv(qc.votes.get(id)),
        }),
        qc.agg_sig.subseteq(r::REPLICAS),
        r::VIEWS.contains(qc.ghost_view),
    }

    pure def justification_shape_inv(justification: Justification): bool = {
        match (justification) {
        | Commit(qc) => commit_qc_shape_inv(qc)
        | Timeout(qc) => timeout_qc_shape_inv(qc)
        }
    }

    pure def proposal_shape_inv(proposal: Proposal): bool = and {
        proposal.block.option_has(block => r::ALL_BLOCKS.contains(block)),
        justification_shape_inv(proposal.justification),
        r::REPLICAS.contains(proposal.sig),
        r::VIEWS.contains(proposal.ghost_block_number),
    }

    pure def new_view_shape_inv(new_view: NewView): bool = and {
        r::REPLICAS.contains(new_view.sig),
        justification_shape_inv(new_view.justification),
    }

    pure def committed_block_shape_inv(cblock: CommittedBlock): bool = and {
        r::ALL_BLOCKS.contains(cblock.block),
        commit_qc_shape_inv(cblock.commit_qc),
    }

    // choose arbitrary values for the initial states
    action init: bool = {
        // assign indices to all replicas in some order
        pure val idx_and_size = r::CORRECT.fold((Map(), 0), (p, r) => (p._1.put(r, p._2), p._2 + 1))
        pure val replica_idx = idx_and_size._1
        pure val correct_size = idx_and_size._2
        pure val all_indices = replica_idx.keys().map(id => replica_idx.get(id))
        all {
            // non-deterministically choose the leader function
            {
                nondet ldr = r::VIEWS.setOfMaps(r::REPLICAS).oneOf()
                r::leader' = ldr
            },
            // non-deterministically choose replica_view and proposer_view
            {
                nondet rv = r::CORRECT.setOfMaps(r::VIEWS).oneOf()
                r::replica_view' = rv
            },
            {
                nondet pv = r::CORRECT.setOfMaps(r::VIEWS).oneOf()
                r::proposer_view' = pv
            },
            // non-deterministically choose replica_state
            nondet replica_views = r::CORRECT.setOfMaps(IVIEWS).oneOf()
            nondet replica_phases =
                r::CORRECT.setOfMaps(Set(PhasePrepare, PhaseTimeout, PhaseCommit)).oneOf()
            nondet high_votes = generate(6, [COMMIT_VOTE_SAMPLE])
            nondet high_votes_is_some = r::REPLICAS.powerset().oneOf()
            nondet high_commit_qcs = generate(6, [COMMIT_QC_SAMPLE])
            nondet high_commit_qcs_is_some = r::REPLICAS.powerset().oneOf()
            nondet high_timeout_qcs = generate(6, [TIMEOUT_QC_SAMPLE])
            nondet high_timeout_qcs_is_some = r::REPLICAS.powerset().oneOf()
            nondet cached_proposals = generate(6, [Set((0, "val_b0"))])
            nondet committed_blocks = generate(6, [[COMMITTED_BLOCK_SAMPLE]])
            all {
                high_votes.length() == correct_size,
                all_indices.forall(i => commit_vote_shape_inv(high_votes[i])),
                high_commit_qcs.length() == correct_size,
                all_indices.forall(i => commit_qc_shape_inv(high_commit_qcs[i])),
                high_timeout_qcs.length() == correct_size,
                all_indices.forall(i => timeout_qc_shape_inv(high_timeout_qcs[i])),
                cached_proposals.length() == correct_size,
                all_indices.forall(i => cached_proposals[i].forall(((bn, bh)) => all {
                    r::VIEWS.contains(bn),
                    r::ALL_BLOCKS.contains(bh),
                })),
                committed_blocks.length() == correct_size,
                all_indices.forall(i => {
                    val committed = committed_blocks[i]
                    committed.indices().forall(j => committed_block_shape_inv(committed[j]))
                }),
                r::replica_state' = r::CORRECT.mapBy(id =>
                    val i = replica_idx.get(id)
                    {
                        view: replica_views.get(id),
                        phase: replica_phases.get(id),
                        high_vote:
                            if (high_votes_is_some.contains(id)) {
                                Some(high_votes[i])
                            } else {
                                COMMIT_VOTE_NONE
                            },
                        high_commit_qc:
                            if (high_commit_qcs_is_some.contains(id)) {
                                Some(high_commit_qcs[i])
                            } else {
                                HIGH_COMMIT_QC_NONE
                            },
                        high_timeout_qc:
                            if (high_timeout_qcs_is_some.contains(id)) {
                                Some(high_timeout_qcs[i])
                            } else {
                                HIGH_TIMEOUT_QC_NONE
                            },
                        cached_proposals: cached_proposals[i],
                        committed_blocks: committed_blocks[i],
                    }
                ),
            },
            // non-deterministical choose msgs_proposal
            {
                nondet msgs_proposal = generate(12, Set(PROPOSAL_SAMPLE))
                all {
                    msgs_proposal.forall(proposal_shape_inv),
                    r::msgs_proposal' = msgs_proposal,
                }
            },
            // non-deterministically choose msgs_signed_commit
            {
                nondet msgs_signed_commit = generate(12, ALL_SIGNED_COMMIT_VOTES)
                all {
                    msgs_signed_commit.forall(signed_commit_vote_shape_inv),
                    r::msgs_signed_commit' = msgs_signed_commit
                }
            },
            // non-deterministically choose msgs_signed_timeout
            {
                nondet msgs_signed_timeout = generate(12, ALL_SIGNED_TIMEOUT_VOTES)
                all {
                    msgs_signed_timeout.forall(signed_timeout_vote_shape_inv),
                    r::msgs_signed_timeout' = msgs_signed_timeout,
                }
            },
            // non-deterministically choose msgs_new_view
            {
                nondet msgs_new_view = generate(12, Set(VIEW_SAMPLE))
                all {
                    msgs_new_view.forall(new_view_shape_inv),
                    r::msgs_new_view' = msgs_new_view,
                }
            },
            // non-deterministically choose store_signed_commit
            {
                nondet store_signed_commit = generate(12, r::REPLICAS.mapBy(id => ALL_SIGNED_COMMIT_VOTES))
                all {
                    store_signed_commit.keys() == r::REPLICAS,
                    r::REPLICAS.forall(id => store_signed_commit.get(id).forall(signed_commit_vote_shape_inv)),
                    r::store_signed_commit' = store_signed_commit,
                }
            },
            // non-deterministically choose store_signed_timeout
            {
                nondet store_signed_timeout = generate(12, r::REPLICAS.mapBy(id => ALL_SIGNED_TIMEOUT_VOTES))
                all {
                    store_signed_timeout.keys() == r::REPLICAS,
                    r::REPLICAS.forall(id => store_signed_timeout.get(id).forall(signed_timeout_vote_shape_inv)),
                    r::store_signed_timeout' = store_signed_timeout,
                }
            },
            // non-deterministically choose ghost_justifications
            {
                nondet ghost_justifications = generate(12, tuples(r::REPLICAS, r::VIEWS).mapBy(((id, v)) => JUSTIFICATION_SAMPLE))
                all {
                    ghost_justifications.keys() == tuples(r::REPLICAS, r::VIEWS),
                    ghost_justifications.keys().forall(((id, v)) => justification_shape_inv(ghost_justifications.get((id, v)))),
                    r::ghost_justifications' = ghost_justifications,
                }
            },
            r::ghost_step' = AnyStep,
        }
    }

    action step: bool = all {
        r::all_invariants,
        r::step,
    }

    val inv = r::all_invariants
}