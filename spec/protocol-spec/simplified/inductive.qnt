module inductive {
    import types.* from "./types"
    import option.* from "../option"
    
    const IWEIGHTS: ReplicaId -> Weight
    const IREPLICA_KEYS: ReplicaId -> ReplicaKey
    const IN: int
    const IF: int
    const ICORRECT: Set[ReplicaId]
    const IFAULTY: Set[ReplicaId]
    const IVIEWS: Set[ViewNumber]
    const IBLOCK_NUMBERS: Set[BlockNumber]
    const IVALID_BLOCKS: Set[Block]
    const IINVALID_BLOCKS: Set[Block]

    pure val IALL_BLOCKS = IVALID_BLOCKS.union(IINVALID_BLOCKS)
    pure val IREPLICAS = ICORRECT.union(IFAULTY)

    var ghost_commit_qc: List[CommitQC]
    var ghost_timeout_qc: List[TimeoutQC]

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
        high_commit_qc_view: Some(0),
    }

    pure val TIMEOUT_QC_SAMPLE: TimeoutQC = {
        votes: Set("n0").mapBy(id => TIMEOUT_VOTE_SAMPLE),
        agg_sig: Set("n0"),
        high_commit_qc: Some(0),
        ghost_view: 0,
    }

    pure def commit_vote_shape_inv(v: CommitVote): bool = and {
        IVIEWS.contains(v.view),
        IBLOCK_NUMBERS.contains(v.block_number),
        IALL_BLOCKS.contains(v.block_hash),
    }

    pure def commit_qc_shape_inv(qc: CommitQC): bool = and {
        commit_vote_shape_inv(qc.vote),
        qc.agg_sig.subseteq(IREPLICAS),
    }

    pure def timeout_vote_shape_inv(v: TimeoutVote): bool = and {
        IVIEWS.contains(v.view),
        v.high_vote.option_has(commit_vote_shape_inv),
        v.high_commit_qc_view.option_has(v_view => IVIEWS.contains(v_view)),
    }    

    pure def timeout_qc_shape_inv(qc: TimeoutQC, commit_qc_size: int): bool = and {
        qc.votes.keys().forall(id => and {
            IREPLICAS.contains(id),
            timeout_vote_shape_inv(qc.votes.get(id)),
        }),
        qc.agg_sig.subseteq(IREPLICAS),
        qc.high_commit_qc.option_has(i => (0 <= i) and (i < commit_qc_size)),
        IVIEWS.contains(qc.ghost_view),
    }

    action init =
        nondet commit_qc = generate(20, [COMMIT_QC_SAMPLE])
        nondet timeout_qc = generate(20, [TIMEOUT_QC_SAMPLE])
        val commit_qc_size = commit_qc.length()
        all {
            // initialize ghost_commit_qc
            ghost_commit_qc' = commit_qc,
            // initialize ghost_timeout_qc
            ghost_timeout_qc' = timeout_qc,
            // restrict the shape of the generated data structures
            ghost_commit_qc.indices().forall(i => commit_qc_shape_inv(ghost_commit_qc[i])),
            ghost_timeout_qc.indices().forall(i => timeout_qc_shape_inv(ghost_timeout_qc[i], commit_qc_size)),
        }

    action step = all {
        // keep ghost_commit_qc unchanged
        ghost_commit_qc' = ghost_commit_qc,
        ghost_timeout_qc' = ghost_timeout_qc,
    }

    /*
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

    // choose arbitrary values for the initial states
    action init: bool = {
        // assign indices to all replicas in some order
        pure val idx_and_size = r::CORRECT.fold((Map(), 0), (p, r) => (p._1.put(r, p._2), p._2 + 1))
        pure val replica_idx = idx_and_size._1
        pure val correct_size = idx_and_size._2
        pure val all_indices = replica_idx.keys().map(id => replica_idx.get(id))
        all {
            { // non-deterministically choose the leader function
                nondet ldr = r::VIEWS.setOfMaps(r::REPLICAS).oneOf()
                r::leader' = ldr
            },
            { // non-deterministically choose replica_view and proposer_view
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
                    and {
                        committed.indices().forall(j => committed_block_shape_inv(committed[j])),
                        committed.indices().subseteq(IBLOCK_NUMBERS),
                    }
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
    */
}