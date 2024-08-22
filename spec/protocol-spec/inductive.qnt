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

    pure val COMMIT_VOTE_SAMPLE: CommitVote = {
        view: 0, block_number: 0, block_hash: "val_b0",
    }

    pure val SIGNED_COMMIT_VOTE_SAMPLE: SignedCommitVote = {
        sig: "n0", vote: COMMIT_VOTE_SAMPLE
    }

    pure val COMMIT_QC_SAMPLE: CommitQC = {
        vote: COMMIT_VOTE_SAMPLE,
        agg_sig: Set("n0"),
    }

    pure val SIGNED_TIMEOUT_VOTE_SAMPLE: SignedTimeoutVote = {
        sig: "n0",
        vote: TIMEOUT_VOTE_SAMPLE,
        high_commit_qc: Some(COMMIT_QC_SAMPLE),
    }

    pure val TIMEOUT_VOTE_SAMPLE: TimeoutVote = {
        view: 0,
        high_vote: Some(COMMIT_VOTE_SAMPLE),
        high_commit_qc_view: Some(0),
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
        high_commit_qc: Some(COMMIT_QC_SAMPLE),
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
        v.high_commit_qc_view.option_has(v_view => r::VIEWS.contains(v_view)),
    }

    pure def signed_timeout_vote_shape_inv(sv: SignedTimeoutVote): bool = and {
        r::REPLICAS.contains(sv.sig),
        timeout_vote_shape_inv(sv.vote),
        sv.high_commit_qc.option_has(qc => commit_qc_shape_inv(qc)),
    }

    pure def timeout_qc_shape_inv(qc: TimeoutQC): bool = and {
        qc.votes.keys().forall(id => and {
            r::REPLICAS.contains(id),
            timeout_vote_shape_inv(qc.votes.get(id)),
        }),
        qc.agg_sig.subseteq(r::REPLICAS),
        qc.high_commit_qc.option_has(hqc => commit_qc_shape_inv(hqc)),
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
                nondet msgs_proposal = generate(6, Set(PROPOSAL_SAMPLE))
                all {
                    msgs_proposal.forall(proposal_shape_inv),
                    r::msgs_proposal' = msgs_proposal,
                }
            },
            // non-deterministically choose msgs_signed_commit
            {
                nondet msgs_signed_commit = generate(6, Set(SIGNED_COMMIT_VOTE_SAMPLE))
                all {
                    msgs_signed_commit.forall(signed_commit_vote_shape_inv),
                    r::msgs_signed_commit' = msgs_signed_commit
                }
            },
            // non-deterministically choose msgs_signed_timeout
            {
                nondet msgs_signed_timeout = generate(6, Set(SIGNED_TIMEOUT_VOTE_SAMPLE))
                all {
                    msgs_signed_timeout.forall(signed_timeout_vote_shape_inv),
                    r::msgs_signed_timeout' = msgs_signed_timeout,
                }
            },
            // non-deterministically choose msgs_new_view
            {
                nondet msgs_new_view = generate(6, Set(VIEW_SAMPLE))
                all {
                    msgs_new_view.forall(new_view_shape_inv),
                    r::msgs_new_view' = msgs_new_view,
                }
            },
            // non-deterministically choose store_signed_commit
            {
                nondet store_signed_commit = generate(6, [Set(SIGNED_COMMIT_VOTE_SAMPLE)])
                all {
                    store_signed_commit.length() == correct_size,
                    store_signed_commit.indices().forall(i => store_signed_commit[i].forall(signed_commit_vote_shape_inv)),
                    r::store_signed_commit' = r::REPLICAS.mapBy(id => store_signed_commit[replica_idx.get(id)]),
                }
            },
            // non-deterministically choose store_signed_timeout
            {
                nondet store_signed_timeout = generate(6, [Set(SIGNED_TIMEOUT_VOTE_SAMPLE)])
                all {
                    store_signed_timeout.length() == correct_size,
                    store_signed_timeout.indices().forall(i => store_signed_timeout[i].forall(signed_timeout_vote_shape_inv)),
                    r::store_signed_timeout' = r::REPLICAS.mapBy(id => store_signed_timeout[replica_idx.get(id)]),
                }
            },
            {
                nondet ghost_justifications = generate(6, [[JUSTIFICATION_SAMPLE]])
                all {
                    ghost_justifications.indices() == all_indices,
                    ghost_justifications.indices().forall(i => {
                        val just = ghost_justifications[i]
                        just.indices().forall(j => {
                            justification_shape_inv(just[j])
                        })
                    }),
                    val pairs = tuples(r::CORRECT, r::VIEWS).filter(((id, v)) => {
                        ghost_justifications[replica_idx.get(id)].indices().contains(v)
                    })
                    r::ghost_justifications' = pairs.mapBy(((id, v)) => {
                        val i = replica_idx.get(id)
                        ghost_justifications[i][v]
                    })
                }
            },
            r::ghost_step' = AnyStep,
        }
    }

    action step: bool = all {
        ind_inv,
        r::step,
    }

    // use this action to check the impact of init on model checking times
    action step_unchanged: bool = all {
        r::all_invariants,
        r::replica_state' = r::replica_state,
        r::msgs_signed_commit' = r::msgs_signed_commit,
        r::msgs_signed_timeout' = r::msgs_signed_timeout,
        r::ghost_justifications' = r::ghost_justifications,
        r::store_signed_commit' = r::store_signed_commit,
        r::store_signed_timeout' = r::store_signed_timeout,
        r::msgs_new_view' = r::msgs_new_view,
        r::msgs_proposal' = r::msgs_proposal,
        r::leader' = r::leader,
        r::replica_view' = r::replica_view,
        r::proposer_view' = r::proposer_view,
        r::ghost_step' = r::ghost_step,
    }

    val ind_inv = all {
        r::store_signed_commit_all_inv,
        r::store_signed_timeout_all_inv,
        r::no_commit_equivocation_inv,
        r::no_timeout_equivocation_inv,
        r::no_new_view_equivocation_inv,
        r::no_proposal_equivocation_inv,
        r::msgs_signed_timeout_inv,
        r::msgs_signed_commit_inv,
        r::justification_is_supported_inv,
        r::one_commit_quorum_inv,
        r::all_replicas_high_commit_qc_inv,
        r::all_replicas_high_timeout_qc_inv,
        r::all_replicas_high_vote_inv,
        r::view_justification_inv,
        r::timeout_high_vote_is_highest_inv,
        r::committed_blocks_have_justification_inv,
        // this invariant is quite expensive!
        //r::one_high_vote_in_timeout_qc_inv,
    }

    val inv = r::all_invariants

    val store_signed_commit_all_inv = r::store_signed_commit_all_inv
    val store_signed_timeout_all_inv = r::store_signed_timeout_all_inv
    val no_commit_equivocation_inv = r::no_commit_equivocation_inv
    val no_timeout_equivocation_inv = r::no_timeout_equivocation_inv
    val no_new_view_equivocation_inv = r::no_new_view_equivocation_inv
    val no_proposal_equivocation_inv = r::no_proposal_equivocation_inv
    val msgs_signed_timeout_inv = r::msgs_signed_timeout_inv
    val msgs_signed_commit_inv = r::msgs_signed_commit_inv
    val justification_is_supported_inv = r::justification_is_supported_inv
    val one_high_vote_in_timeout_qc_inv = r::one_high_vote_in_timeout_qc_inv
    val one_commit_quorum_inv = r::one_commit_quorum_inv
    val all_replicas_high_commit_qc_inv = r::all_replicas_high_commit_qc_inv
    val all_replicas_high_timeout_qc_inv = r::all_replicas_high_timeout_qc_inv
    val all_replicas_high_vote_inv = r::all_replicas_high_vote_inv
    val view_justification_inv = r::view_justification_inv
    val timeout_high_vote_is_highest_inv = r::timeout_high_vote_is_highest_inv
    val committed_blocks_have_justification_inv = r::committed_blocks_have_justification_inv
}
