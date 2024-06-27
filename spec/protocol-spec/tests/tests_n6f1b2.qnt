// tests for the configuration of: N = 6, F = 1, and 2 Byzantine faults
module tests_n6f1b2 {
    import types.* from "../types"
    import defs.* from "../defs"
    import option.* from "../option"
    
    import replica(
        CORRECT = Set("n0", "n1", "n2", "n3"),
        FAULTY = Set("n4", "n5"),
        WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
        REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5"),
        N = 6,
        F = 1,
        VIEWS = 0.to(5),
        VALID_BLOCKS = Set("val_b0", "val_b1"),
        INVALID_BLOCKS = Set("inv_b3")
    ).* from "../replica"

    // an initial quorum certificate
    pure val init_timeout_qc: TimeoutQC = {
        votes: REPLICAS.mapBy(id => { view: 0, high_vote: COMMIT_VOTE_NONE, high_commit_qc: HIGH_COMMIT_QC_NONE}),
        agg_sig: REPLICAS,
        ghost_view: 0,
    }

    def replica_committed(id: ReplicaId): List[Block] = {
        replica_state.get(id).committed_blocks.foldl([], (bs, cb) => bs.append(cb.block))
    }

    // This test demonstrates that two Byzantine replicas can convince three correct replicas
    // to finalize two different blocks.
    // 29 steps.
    run disagreement_Test = {
        val high_vote_b0 = Some({ view: 1, block_number: 0, block_hash: "val_b0" })
        val high_vote_b1 = Some({ view: 1, block_number: 0, block_hash: "val_b1" })
        val timeout_vote_b0: TimeoutVote = { view: 1, high_commit_qc: None, high_vote: high_vote_b0 }
        val timeout_vote_b1: TimeoutVote = { view: 1, high_commit_qc: None, high_vote: high_vote_b1 }
        // n4 and n5 partition the correct replicas into n0, n1, n2 and n1, n2, n3
        val timeout_qc: TimeoutQC = {
            ghost_view: 1,
            agg_sig: Set("n1", "n2", "n3", "n4", "n5"),
            votes: Set("n1", "n2", "n3", "n4", "n5").mapBy(id => {
                if (Set("n1", "n2").contains(id)) {
                    timeout_vote_b0
                } else {
                    timeout_vote_b1
                }
            })
        }
        val new_view_timeout: NewView = {
            sig: "n4",
            justification: Timeout(timeout_qc),
        }
        val proposal_b0_in_1: Proposal = {
            block: Some("val_b0"),
            justification: Timeout(init_timeout_qc),
            sig: "n4",
            ghost_block_number: 0,
        }
        val proposal_b1_in_1: Proposal = {
            block: Some("val_b1"),
            justification: Timeout(init_timeout_qc),
            sig: "n4",
            ghost_block_number: 0,
        }
        val proposal_b1_in_2: Proposal = {
            block: None,
            justification: new_view_timeout.justification,
            sig: "n5",
            ghost_block_number: 0,
        }
        init_view_1_with_leader(Map(0 -> "n0", 1 -> "n4", 2 -> "n5", 3 -> "n2", 4 -> "n3", 5 -> "n5"))
             // the faulty leader n4 of view 1 proposes val_b0
            .then(all {
                msgs_proposal' = msgs_proposal.union(Set(proposal_b0_in_1)),
                unchanged_all_but_proposal,
            })
            .expect(all_invariants)
            // n0, n1, n2 receive the proposal
            .then(all { on_proposal("n0", proposal_b0_in_1), unchanged_leader_replica })
            .then(all { on_proposal("n1", proposal_b0_in_1), unchanged_leader_replica })
            .then(all { on_proposal("n2", proposal_b0_in_1), unchanged_leader_replica })
            .expect(all_invariants)
            // the faulty replicas n4 and n5 send commit votes for val_b0
            .then(all {
                val commit_vote_n4: SignedCommitVote = {
                    vote: { block_hash: "val_b0", block_number: 0, view: 1 },
                    sig: "n4",
                }
                val commit_vote_n5: SignedCommitVote = {
                    vote: { block_hash: "val_b0", block_number: 0, view: 1 },
                    sig: "n5",
                }
                msgs_signed_commit' = msgs_signed_commit.union(Set(commit_vote_n4, commit_vote_n5)),
                unchanged_all_but_commit_vote,
            })
            // the correct replica n0 finalizes the block val_b0
            .then(replica_on_commit("n0", "val_b0", 0, 1, "n0"))
            .then(replica_on_commit("n0", "val_b0", 0, 1, "n1"))
            .then(replica_on_commit("n0", "val_b0", 0, 1, "n2"))
            .then(replica_on_commit("n0", "val_b0", 0, 1, "n4"))
            .then(replica_on_commit("n0", "val_b0", 0, 1, "n5"))
            .expect(all_invariants)
            .then(all {
                assert(replica_committed("n0") == [ "val_b0" ]),
                unchanged_all,
            })
            // the faulty leader n4 of view 1 proposes again, but this time it proposes val_b1
            .then(all {
                msgs_proposal' = msgs_proposal.union(Set(proposal_b1_in_1)),
                unchanged_all_but_proposal,
            })
            .expect(all_invariants)
            // n3 receives the second proposal and sets its high_vote to val_b1
            .then(all { on_proposal("n3", proposal_b1_in_1), unchanged_leader_replica })
            .expect(all_invariants)
            // n1, n2, n3 time out
            .then(all { on_timer_is_finished("n1"), unchanged_leader_replica })
            .then(all { on_timer_is_finished("n2"), unchanged_leader_replica })
            .then(all { on_timer_is_finished("n3"), unchanged_leader_replica })
            // n4 and n5 send SignedTimeoutVote and NewView
            .then(all {
                val timeout_vote_n4: SignedTimeoutVote = {
                    vote: timeout_vote_b1,
                    sig: "n4",
                }
                val timeout_vote_n5: SignedTimeoutVote = {
                    vote: timeout_vote_b1,
                    sig: "n5",
                }
                all {
                    msgs_signed_timeout' = msgs_signed_timeout.union(Set(timeout_vote_n4, timeout_vote_n5)),
                    msgs_new_view' = msgs_new_view.union(Set(new_view_timeout)),
                    unchanged_all_but_timeout_vote_and_new_view,
                }
            })
            // n1, n2, n3 receive NewView and start view 2
            .then(all { on_new_view("n1", new_view_timeout), unchanged_leader_replica })
            .then(all { on_new_view("n2", new_view_timeout), unchanged_leader_replica })
            .then(all { on_new_view("n3", new_view_timeout), unchanged_leader_replica })
            // n5 re-proposes val_b1 in view 2
            .then(all {
                msgs_proposal' = msgs_proposal.union(Set(proposal_b1_in_2)),
                unchanged_all_but_proposal,
            })
            .expect(all_invariants)
            // n1, n2, n3 receive the re-proposal for val_b1 and send their commit votes
            .then(all { on_proposal("n1", proposal_b1_in_2), unchanged_leader_replica })
            .then(all { on_proposal("n2", proposal_b1_in_2), unchanged_leader_replica })
            .then(all { on_proposal("n3", proposal_b1_in_2), unchanged_leader_replica })
            .expect(all_invariants)
            // n4 and n5 send SignedCommitVote for val_b1 in view 2
            .then(all {
                val commit_vote_n4: SignedCommitVote = {
                    vote: { block_hash: "val_b1", block_number: 0, view: 2 },
                    sig: "n4",
                }
                val commit_vote_n5: SignedCommitVote = {
                    vote: { block_hash: "val_b1", block_number: 0, view: 2 },
                    sig: "n5",
                }
                msgs_signed_commit' = msgs_signed_commit.union(Set(commit_vote_n4, commit_vote_n5)),
                unchanged_all_but_commit_vote,
            })
            // the correct replica n3 received 4 on_commit votes
            .then(replica_on_commit("n3", "val_b1", 0, 2, "n1"))
            .then(replica_on_commit("n3", "val_b1", 0, 2, "n2"))
            .then(replica_on_commit("n3", "val_b1", 0, 2, "n3"))
            .then(replica_on_commit("n3", "val_b1", 0, 2, "n4"))
            .expect(all_invariants)
            // this last on_commit finalizes the block val_b1 in view 2 and breaks our invariants
            .then(replica_on_commit("n3", "val_b1", 0, 2, "n5"))
            .then(all {
                assert(replica_committed("n0") == [ "val_b0" ]),
                assert(replica_committed("n3") == [ "val_b1" ]),
                // agreement is violated, as expected
                assert(not(agreement_inv)),
                unchanged_all,
            })
    }
    
    action replica_on_commit(id: ReplicaId, block_hash: BlockHash, block_number: BlockNumber, view: ViewNumber, sig: ReplicaKey): bool = all {
        on_commit(id, {
            vote: { block_hash: block_hash, block_number: block_number, view: view },
            sig: sig
        }),
        unchanged_leader_replica,
    }

    action unchanged_leader_replica = all {
        leader' = leader,
        replica_view' = replica_view,
    }

    action unchanged_all = all {
        msgs_signed_timeout' = msgs_signed_timeout, msgs_new_view' = msgs_new_view,
        msgs_proposal' = msgs_proposal, msgs_signed_commit' = msgs_signed_commit,
        store_signed_timeout' = store_signed_timeout, store_signed_commit' = store_signed_commit,
        proposer_view' = proposer_view, ghost_justifications' = ghost_justifications,
        ghost_step' = ghost_step,
        leader' = leader,
        replica_view' = replica_view, replica_state' = replica_state,
    }

    action unchanged_all_but_proposal = all {
        msgs_signed_timeout' = msgs_signed_timeout, msgs_new_view' = msgs_new_view,
        msgs_signed_commit' = msgs_signed_commit,
        store_signed_timeout' = store_signed_timeout, store_signed_commit' = store_signed_commit,
        proposer_view' = proposer_view, ghost_justifications' = ghost_justifications,
        ghost_step' = ghost_step,
        leader' = leader,
        replica_view' = replica_view, replica_state' = replica_state,
    }

    action unchanged_all_but_commit_vote = all {
        msgs_signed_timeout' = msgs_signed_timeout, msgs_new_view' = msgs_new_view,
        msgs_proposal' = msgs_proposal,
        store_signed_timeout' = store_signed_timeout, store_signed_commit' = store_signed_commit,
        proposer_view' = proposer_view, ghost_justifications' = ghost_justifications,
        ghost_step' = ghost_step,
        leader' = leader,
        replica_view' = replica_view, replica_state' = replica_state,
    }

    action unchanged_all_but_timeout_vote = all {
        msgs_new_view' = msgs_new_view,
        msgs_proposal' = msgs_proposal, msgs_signed_commit' = msgs_signed_commit,
        store_signed_timeout' = store_signed_timeout, store_signed_commit' = store_signed_commit,
        proposer_view' = proposer_view, ghost_justifications' = ghost_justifications,
        ghost_step' = ghost_step,
        leader' = leader,
        replica_view' = replica_view, replica_state' = replica_state,
    }

    action unchanged_all_but_timeout_vote_and_new_view = all {
        msgs_proposal' = msgs_proposal, msgs_signed_commit' = msgs_signed_commit,
        store_signed_timeout' = store_signed_timeout, store_signed_commit' = store_signed_commit,
        proposer_view' = proposer_view, ghost_justifications' = ghost_justifications,
        ghost_step' = ghost_step,
        leader' = leader,
        replica_view' = replica_view, replica_state' = replica_state,
    }
}