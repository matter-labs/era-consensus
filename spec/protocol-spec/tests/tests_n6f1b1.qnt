// tests for the configuration of: N = 6, F = 1, and 1 Byzantine faults
module tests {
    import types.* from "../types"
    import defs.* from "../defs"
    import option.* from "../option"
    import replica(
        CORRECT = Set("n0", "n1", "n2", "n3", "n4"),
        FAULTY = Set("n5"),
        WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
        REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5"),
        N = 6,
        F = 1,
        VIEWS = 0.to(5),
        VALID_BLOCKS = Set("val_b0", "val_b1", "val_b2"),
        INVALID_BLOCKS = Set("inv_b3")
    ).* from "../replica"

    // an initial quorum certificate from all replicas
    pure val init_timeout_qc: TimeoutQC = {
        votes: CORRECT.mapBy(id => { view: 0, high_vote: COMMIT_VOTE_NONE, high_commit_qc_view: VIEW_NUMBER_NONE }),
        agg_sig: CORRECT,
        ghost_view: 0,
        high_commit_qc: HIGH_COMMIT_QC_NONE,
    }

    // an initial quorum certificate from the correct replicas
    val init_timeout_corr_qc: TimeoutQC = {
        votes: CORRECT.mapBy(id => { view: 0, high_vote: None, high_commit_qc_view: VIEW_NUMBER_NONE }),
        agg_sig: CORRECT,
        ghost_view: 0,
        high_commit_qc: HIGH_COMMIT_QC_NONE,
    }

    def replica_committed(id: ReplicaId): List[Block] = {
        replica_state.get(id).committed_blocks.foldl([], (bs, cb) => bs.append(cb.block))
    }

    run replicas_bootstrap_in_view0_Test = {
        pure val timeout_votes0 = CORRECT.map(id => {
            sig: sig_of_id(id),
            vote: { view: 0, high_vote: COMMIT_VOTE_NONE, high_commit_qc_view: VIEW_NUMBER_NONE },
            high_commit_qc: HIGH_COMMIT_QC_NONE, 
        })
        init_view_0_with_leader(Map(0 -> "n0", 1 -> "n0", 2 -> "n1", 3 -> "n2", 4 -> "n3"))
            .then(replica_receives_all_timeout_votes("n0", COMMIT_VOTE_NONE, VIEW_NUMBER_NONE, HIGH_COMMIT_QC_NONE, 0))
            .expect(all_invariants)
            .then(replica_receives_new_view("n1", Timeout(init_timeout_corr_qc)))
            .expect(all_invariants)
            .then(replica_receives_new_view("n1", Timeout(init_timeout_corr_qc)))
            .expect(all_invariants)
            .then(replica_receives_new_view("n2", Timeout(init_timeout_corr_qc)))
            .expect(all_invariants)
            .then(replica_receives_new_view("n3", Timeout(init_timeout_corr_qc)))
            .expect(all_invariants)
            .then(replica_receives_new_view("n4", Timeout(init_timeout_corr_qc)))
            .expect(all_invariants)
            .then(all {
                CORRECT.forall(id => {
                    val state = replica_state.get(id)
                    all {
                        assert(state.phase == PhasePrepare),
                        assert(state.high_timeout_qc == Some(init_timeout_corr_qc)),
                        assert(state.high_vote == COMMIT_VOTE_NONE),
                        assert(state.high_commit_qc == HIGH_COMMIT_QC_NONE),
                    }
                }),
                assert(replica_view == CORRECT.mapBy(_ => 0)),
                assert(proposer_view == CORRECT.mapBy(_ => 0)),
                assert(msgs_signed_timeout == timeout_votes0),
                assert(msgs_new_view == CORRECT.map(id => {
                        { sig: id, justification: Timeout(init_timeout_corr_qc) }
                    })),
                assert(msgs_signed_commit == Set()),
                assert(msgs_proposal == Set()),
                assert(store_signed_timeout == CORRECT.mapBy(id => {
                        if (id == "n0") timeout_votes0 else Set()
                    })),
                assert(store_signed_commit == CORRECT.mapBy(id => Set())),
                assert(CORRECT.forall(id => replica_state.get(id).cached_proposals == Set())),
                assert(ghost_justifications ==
                    tuples(CORRECT, Set(1)).mapBy(((id, view)) => Timeout(init_timeout_corr_qc))),
                assert(CORRECT.forall(id => replica_state.get(id).committed_blocks == [])),
                assert(all_replicas_high_timeout_qc_inv),
                unchanged_all,
            })
        .expect(all_invariants)
    }
    
    run replicas_in_view1_Test = {
        init_view_1_with_leader(Map(0 -> "n0", 1 -> "n0", 2 -> "n1", 3 -> "n2", 4 -> "n3"))
            .then(all {
                assert(all_replicas_high_timeout_qc_inv),
                unchanged_all,
            })
            .expect(all_invariants)
    }

    // the leaders propose a block and all replicas commit that block
    run replicas_normal_case_Test = {
        init_view_1_with_leader(Map(0 -> "n0", 1 -> "n0", 2 -> "n1", 3 -> "n2", 4 -> "n3"))
            // the leader proposes
            .then(all {
                proposer_step("n0", "val_b0"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            // replicas process the propose message
            .then(all_replicas_get_propose("val_b0", 0, "n0", Timeout(init_timeout_qc)))
            .expect(all_invariants)
            // n0 process the commit messages
            .then(replica_commits("n0", "val_b0", 0, 1))
            .expect(all_invariants)
            // n1 process the commit messages
            .then(replica_commits("n1", "val_b0", 0, 1))
            .expect(all_invariants)
            // n2 process the commit messages
            .then(replica_commits("n2", "val_b0", 0, 1))
            .expect(all_invariants)
            // n3 process the commit messages
            .then(replica_commits("n3", "val_b0", 0, 1))
            .expect(all_invariants)
            // n4 process the commit messages
            .then(replica_commits("n4", "val_b0", 0, 1))
            .expect(all_invariants)
            .then(all {
                assert(replica_committed("n0") == ["val_b0"]),
                assert(replica_committed("n1") == ["val_b0"]),
                assert(replica_committed("n2") == ["val_b0"]),
                assert(replica_committed("n3") == ["val_b0"]),
                assert(replica_committed("n4") == ["val_b0"]),
                unchanged_all,
            })
            .expect(all_invariants)
            // the new leader proposes    
            .then(all {
                proposer_step("n1", "val_b1"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            // replicas process the propose message
            .then(all_replicas_get_propose("val_b1", 1, "n1",
                Commit({
                    vote:{view: 1, block_number: 0, block_hash: "val_b0"},
                    agg_sig: Set("n0", "n1", "n2", "n3", "n4")
                })))
            .expect(all_invariants)
            // n0 process the commit messages
            .then(replica_commits("n0", "val_b1", 1, 2))
            .expect(all_invariants)
            // n1 process the commit messages
            .then(replica_commits("n1", "val_b1", 1, 2))
            .expect(all_invariants)
            // n2 process the commit messages
            .then(replica_commits("n2", "val_b1", 1, 2))
            .expect(all_invariants)
            // n3 process the commit messages
            .then(replica_commits("n3", "val_b1", 1, 2))
            .expect(all_invariants)
            // n4 process the commit messages
            .then(replica_commits("n4", "val_b1", 1, 2))
            .expect(all_invariants)
            .then(all {
                assert(replica_committed("n0") == [ "val_b0", "val_b1" ]),
                assert(replica_committed("n1") == [ "val_b0", "val_b1" ]),
                assert(replica_committed("n2") == [ "val_b0", "val_b1" ]),
                assert(replica_committed("n3") == [ "val_b0", "val_b1" ]),
                assert(replica_committed("n4") == [ "val_b0", "val_b1" ]),
                unchanged_all,
            })
            .expect(all_invariants)
            // the new leader proposes    
            .then(all {
                proposer_step("n2", "val_b2"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            // replicas process the propose message
            .then(all_replicas_get_propose("val_b2", 2, "n2",
                Commit({
                    vote: {
                        view: 2,
                        block_number: 1,
                        block_hash: "val_b1",
                    },
                    agg_sig: Set("n0", "n1", "n2", "n3", "n4")
                })))
            .expect(all_invariants)
            // n0 process the commit messages
            .then(replica_commits("n0", "val_b2", 2, 3))
            .expect(all_invariants)
            // n1 process the commit messages
            .then(replica_commits("n1", "val_b2", 2, 3))
            .expect(all_invariants)
            // n2 process the commit messages
            .then(replica_commits("n2", "val_b2", 2, 3))
            .expect(all_invariants)
            // n3 process the commit messages
            .then(replica_commits("n3", "val_b2", 2, 3))
            .expect(all_invariants)
            // n4 process the commit messages
            .then(replica_commits("n4", "val_b2", 2, 3))
            .expect(all_invariants)
            .then(all {
                assert(replica_committed("n0") == [ "val_b0", "val_b1", "val_b2" ]),
                assert(replica_committed("n1") == [ "val_b0", "val_b1", "val_b2" ]),
                assert(replica_committed("n2") == [ "val_b0", "val_b1", "val_b2" ]),
                assert(replica_committed("n3") == [ "val_b0", "val_b1", "val_b2" ]),
                assert(replica_committed("n4") == [ "val_b0", "val_b1", "val_b2" ]),
                unchanged_all,
            })
            .expect(all_invariants)
    }

    run view0_change_in_view0_Test = {
        init_view_1_with_leader(Map(0 -> "n0", 1 -> "n0", 2 -> "n1", 3 -> "n2", 4 -> "n3"))
             // the leader proposes
            .then(all {
                proposer_step("n0", "val_b0"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            // all messages are lost
            .then(all {
                on_timer_is_finished("n1"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                on_timer_is_finished("n2"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                on_timer_is_finished("n3"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                on_timer_is_finished("n4"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            // all replicas send TimeoutVote to each other, each replica gets all messages from all other replicas including itself
            .then(replica_receives_all_timeout_votes("n0", COMMIT_VOTE_NONE, VIEW_NUMBER_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(replica_receives_all_timeout_votes("n1", COMMIT_VOTE_NONE, VIEW_NUMBER_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(replica_receives_all_timeout_votes("n2", COMMIT_VOTE_NONE, VIEW_NUMBER_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(replica_receives_all_timeout_votes("n3", COMMIT_VOTE_NONE, VIEW_NUMBER_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(replica_receives_all_timeout_votes("n4", COMMIT_VOTE_NONE, VIEW_NUMBER_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(all {
                proposer_step("n1", "val_b1"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                assert(replica_state.get("n0").high_timeout_qc.option_has(qc => qc.votes.keys().size()==5)),
                assert(replica_state.get("n1").high_timeout_qc.option_has(qc => qc.votes.keys().size()==5)),
                assert(replica_state.get("n2").high_timeout_qc.option_has(qc => qc.votes.keys().size()==5)),
                assert(replica_state.get("n3").high_timeout_qc.option_has(qc => qc.votes.keys().size()==5)),
                assert(replica_state.get("n4").high_timeout_qc.option_has(qc => qc.votes.keys().size()==5)),

                assert(replica_state.get("n0").high_timeout_qc.option_has(qc => qc.agg_sig.size()==5)),
                assert(replica_state.get("n1").high_timeout_qc.option_has(qc => qc.agg_sig.size()==5)),
                assert(replica_state.get("n2").high_timeout_qc.option_has(qc => qc.agg_sig.size()==5)),
                assert(replica_state.get("n3").high_timeout_qc.option_has(qc => qc.agg_sig.size()==5)),
                assert(replica_state.get("n4").high_timeout_qc.option_has(qc => qc.agg_sig.size()==5)),
                unchanged_all,
            })
            .expect(all_invariants)
    }

    action all_replicas_get_propose(block: Block, block_number: BlockNumber,
            leader: ReplicaId, justification: Justification): bool = {
        val proposal: Proposal = {
            block: Some(block),
            sig: leader,
            justification: justification,
            ghost_block_number: block_number,
        }
        all {
            on_proposal("n0", proposal),
            unchanged_leader_replica,
        }.then(all {
            on_proposal("n1", proposal),
            unchanged_leader_replica,
        })
        .then(all {
            on_proposal("n2", proposal),
            unchanged_leader_replica,
        })
        .then(all {
            on_proposal("n3", proposal),
            unchanged_leader_replica,
        })
        .then(all {
            on_proposal("n4", proposal),
            unchanged_leader_replica,
        })
    }

    action replica_receives_all_timeout_votes(id: ReplicaId, high_vote: Option[CommitVote],
            high_commit_qc_view: Option[ViewNumber], high_commit_qc: Option[CommitQC], view: ViewNumber): bool = {
        val timeout_vote: TimeoutVote = { view: view, high_commit_qc_view: high_commit_qc_view, high_vote: high_vote }
        all {
            on_timeout(id, { vote: timeout_vote, high_commit_qc: high_commit_qc, sig: "n0" }),
            unchanged_leader_replica,
        }
        .then(all {
            on_timeout(id, { vote: timeout_vote, high_commit_qc: high_commit_qc, sig: "n1" }),
            unchanged_leader_replica,
        })
        .then(all {
            on_timeout(id, { vote: timeout_vote, high_commit_qc: high_commit_qc, sig: "n2" }),
            unchanged_leader_replica,
        })
        .then(all {
            on_timeout(id, { vote: timeout_vote, high_commit_qc: high_commit_qc, sig: "n3" }),
            unchanged_leader_replica,
        })
        .then(all {
            on_timeout(id, { vote: timeout_vote, high_commit_qc: high_commit_qc, sig: "n4" }),
            unchanged_leader_replica,
        })
    }

    action replica_commits(id: ReplicaId, block_hash: BlockHash, block_number: BlockNumber, view: ViewNumber):bool = all {
        on_commit(id, { vote: {block_hash: block_hash, block_number: block_number, view: view }, sig:"n0"}),
        unchanged_leader_replica,
    }.then(all {
        on_commit(id, { vote: {block_hash: block_hash, block_number: block_number, view: view }, sig:"n1"}),
        unchanged_leader_replica,
    }).then(all {
        on_commit(id, { vote: {block_hash: block_hash, block_number: block_number, view: view }, sig:"n2"}),
        unchanged_leader_replica,
    }).then(all {
        on_commit(id, { vote: {block_hash: block_hash, block_number: block_number, view: view }, sig:"n3"}),
        unchanged_leader_replica,
    }).then(all {
        on_commit(id, { vote: {block_hash: block_hash, block_number: block_number, view: view }, sig:"n4"}),
        unchanged_leader_replica,
    })

    action replica_receives_new_view(id: ReplicaId, justification: Justification): bool = all {
        on_new_view(id, { sig: id, justification: Timeout(init_timeout_corr_qc) }),
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
}