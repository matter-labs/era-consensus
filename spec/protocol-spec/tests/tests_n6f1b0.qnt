// tests for the configuration of: N = 6, F = 1, and 0 Byzantine faults
module tests_n6f1b0 {
    import types.* from "../types"
    import defs.* from "../defs"
    import option.* from "../option"
    
    import replica(
        CORRECT = Set("n0", "n1", "n2", "n3", "n4", "n5"),
        FAULTY = Set(),
        WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
        REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5"),
        N = 6,
        F = 1,
        VIEWS = 0.to(5),
        VALID_BLOCKS = Set("val_b0", "val_b1", "val_b2"),
        INVALID_BLOCKS = Set("inv_b3")
    ).* from "../replica"

    // an initial quorum certificate
    pure val init_timeout_qc: TimeoutQC = {
        votes: CORRECT.mapBy(id => { view: 0, high_vote: COMMIT_VOTE_NONE, high_commit_qc: HIGH_COMMIT_QC_NONE}),
        agg_sig: CORRECT,
        ghost_view: 0,
    }

    def replica_committed(id: ReplicaId): List[Block] = {
        replica_state.get(id).committed_blocks.foldl([], (bs, cb) => bs.append(cb.block))
    }

    // The case of 6 correct replicas, but one of them being slow.
    // This requires 56 steps.
    run replicas_one_slow_Test = {
        init_view_1_with_leader(Map(0 -> "n0", 1 -> "n0", 2 -> "n1", 3 -> "n2", 4 -> "n3"))
             // the leader of view 0 proposes
            .then(all {
                proposer_step("n0", "val_b0"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            // replicas process the propose message
            .then(five_replicas_get_propose("val_b0", 0, "n0", Timeout(init_timeout_qc)))
            .expect(all_invariants)
            // the processes n0, n1, n2, n3, n4 finalize the block
            .then(replica_commits("n0", "val_b0", 0, 1))
            .expect(all_invariants)
            .then(replica_commits("n1", "val_b0", 0, 1))
            .expect(all_invariants)
            .then(replica_commits("n2", "val_b0", 0, 1))
            .expect(all_invariants)
            .then(replica_commits("n3", "val_b0", 0, 1))
            .expect(all_invariants)
            .then(replica_commits("n4", "val_b0", 0, 1))
            .expect(all_invariants)
            // the leader of view 1 proposes    
            .then(all {
                proposer_step("n1", "val_b1"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            // replicas process the propose message
            .then(five_replicas_get_propose("val_b1", 1, "n1",
                Commit({
                    vote: {
                        view: 1, block_number: 0, block_hash: "val_b0",
                    },
                    agg_sig: Set("n0", "n1", "n2", "n3", "n4")
                })))
            .expect(all_invariants)
            // the processes n0, n1, n2, n3, n4 finalize the block
            .then(replica_commits("n0", "val_b1", 1, 2))
            .expect(all_invariants)
            .then(replica_commits("n1", "val_b1", 1, 2))
            .expect(all_invariants)
            .then(replica_commits("n2", "val_b1", 1, 2))
            .expect(all_invariants)
            .then(replica_commits("n3", "val_b1", 1, 2))
            .expect(all_invariants)
            .then(replica_commits("n4", "val_b1", 1, 2))
            .expect(all_invariants)
            .then(all {
                assert(replica_committed("n0") == ["val_b0", "val_b1"]),
                assert(replica_committed("n1") == ["val_b0", "val_b1"]),
                assert(replica_committed("n2") == ["val_b0", "val_b1"]),
                assert(replica_committed("n3") == ["val_b0", "val_b1"]),
                assert(replica_committed("n4") == ["val_b0", "val_b1"]),
                unchanged_all,
            })
            .expect(all_invariants)
            // the slow replica n5 tries to catch up and receives the message for view 2
            .then(replica_commits("n5", "val_b1", 1, 2))
            .expect(all_invariants)
            // Now the slow replica n5 has block number 1 but no block number 0.
            // Moreover, n5 cannot receive block number 0 any longer.
            .then(all {
                assert(replica_state.get("n5").view == 3),
                assert(replica_committed("n0") == ["val_b0", "val_b1"]),
                assert(replica_committed("n1") == ["val_b0", "val_b1"]),
                assert(replica_committed("n2") == ["val_b0", "val_b1"]),
                assert(replica_committed("n3") == ["val_b0", "val_b1"]),
                assert(replica_committed("n4") == ["val_b0", "val_b1"]),
                // the slow replica does not finalize the block, even though it has committed it
                assert(replica_committed("n5") == []),
                all_invariants,
                unchanged_all,
            })
    }

    run fab_bad_scenario_Test = {

        val proposal_l5_v1_b0: Proposal = {
            block: Some("val_b0"),
            sig: "n5",
            justification: Timeout(init_timeout_qc),
            ghost_block_number: 0,
        }
    
        val commit_vote_v1_b0: CommitVote = {
            view: 1,
            block_number: 0,
            block_hash: "val_b0"
        }
        val timeout_qc_v1: TimeoutQC = {
            votes: Map(
                "n0" -> { view: 2, high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(commit_vote_v1_b0) },
                "n1" -> { view: 2, high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(commit_vote_v1_b0) },
                "n2" -> { view: 2, high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(commit_vote_v1_b0) },
                "n3" -> { view: 2, high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(commit_vote_v1_b0) },
                "n5" -> { view: 2, high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(commit_vote_v1_b0) }
            ), 
            agg_sig: Set("n0", "n1", "n2", "n3", "n5"),
            ghost_view: 2
        }
        init_view_1_with_leader(Map(0 -> "n0", 1 -> "n5", 2 -> "n1", 3 -> "n2", 4 -> "n3", 5 -> "n4"))
            // the leader proposes, all replicas in view=1
            .then(all {
                replica_state.get("n0").view==1, unchanged_all
            })
            .expect(all_invariants)
            .then(all {
                proposer_step("n5", "val_b0"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                on_proposal("n0", proposal_l5_v1_b0),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                on_proposal("n1", proposal_l5_v1_b0),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                on_proposal("n2", proposal_l5_v1_b0),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                on_proposal("n3", proposal_l5_v1_b0),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                on_proposal("n5", proposal_l5_v1_b0),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                on_commit("n1", {sig: "n0", vote: commit_vote_v1_b0}),
                unchanged_leader_replica
            })
            .expect(all_invariants)
            .then(all {
                on_commit("n1", {sig: "n1", vote: commit_vote_v1_b0}),
                unchanged_leader_replica
            })
            .expect(all_invariants)
            .then(all {
                on_commit("n1", {sig: "n2", vote: commit_vote_v1_b0}),
                unchanged_leader_replica
            })
            .expect(all_invariants)
            .then(all {
                on_commit("n1", {sig: "n3", vote: commit_vote_v1_b0}),
                unchanged_leader_replica
            })
            .expect(all_invariants)
            .then(all {
                on_commit("n1", {sig: "n5", vote: commit_vote_v1_b0}),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                replica_state.get("n0").view==1, unchanged_all
            })
            .expect(all_invariants)
            .then(all {
                replica_state.get("n1").view==2, unchanged_all
            })
            .expect(all_invariants)
            // Delay all further messages until new view starts.
            .then(all {
                on_timer_is_finished("n0"),
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
            .then(all {
                on_timer_is_finished("n5"),
                unchanged_leader_replica,
            })
            .expect(all_invariants)
            .then(all {
                replica_state.get("n0").view==1, unchanged_all
            })
            .expect(all_invariants)
            .then(all {
                replica_state.get("n0").high_commit_qc==HIGH_COMMIT_QC_NONE, unchanged_all
            })
            .expect(all_invariants)
            .then(all {
                replica_state.get("n0").high_vote==Some(commit_vote_v1_b0), unchanged_all
            })
            .expect(all_invariants)
            // replica n1 has high_vote and high_commit_qc
            .then(all {
                replica_state.get("n1").view==2, unchanged_all
            })
            .expect(all_invariants)
            .then(all {
                replica_state.get("n1").high_vote==Some(commit_vote_v1_b0), unchanged_all
            })
            .expect(all_invariants)
            .then(all {
                replica_state.get("n1").high_commit_qc==Some({ vote: commit_vote_v1_b0, agg_sig: Set("n0", "n1", "n2", "n3", "n5")}), unchanged_all
            })
            .expect(all_invariants)
            .then(replica_receives_all_timeout_votes("n0", COMMIT_VOTE_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(replica_receives_all_timeout_votes("n2", COMMIT_VOTE_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(replica_receives_all_timeout_votes("n3", COMMIT_VOTE_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(replica_receives_all_timeout_votes("n4", COMMIT_VOTE_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(replica_receives_all_timeout_votes("n5", COMMIT_VOTE_NONE, HIGH_COMMIT_QC_NONE, 1))
            .expect(all_invariants)
            .then(all {
                proposer_step("n1", "val_b1"),
                unchanged_leader_replica,
            })
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

    action replica_receives_all_timeout_votes(id: ReplicaId, high_vote: Option[CommitVote], high_commit_qc: Option[CommitQC], view: ViewNumber):bool = all {
        on_timeout(id, {vote: { view: view, high_commit_qc: high_commit_qc, high_vote: high_vote}, sig: "n0"} ),
        unchanged_leader_replica,
    }
    .then(all {
        on_timeout(id, {vote: { view: view, high_commit_qc: high_commit_qc, high_vote: high_vote}, sig: "n1"} ),
        unchanged_leader_replica,
    })
    .then(all {
        on_timeout(id, {vote: { view: view, high_commit_qc: high_commit_qc, high_vote: high_vote}, sig: "n2"} ),
        unchanged_leader_replica,
    })
    .then(all {
        on_timeout(id, {vote: { view: view, high_commit_qc: high_commit_qc, high_vote: high_vote}, sig: "n3"} ),
        unchanged_leader_replica,
    })
    .then(all {
        on_timeout(id, {vote: { view: view, high_commit_qc: high_commit_qc, high_vote: high_vote}, sig: "n4"} ),
        unchanged_leader_replica,
    })

    action five_replicas_get_propose(block: Block, block_number: BlockNumber,
            leader: ReplicaId, justification: Justification): bool = {
        pure val proposal = {
            block: Some(block),
            sig: leader,
            justification: justification,
            ghost_block_number: block_number
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

    action unchanged_leader_replica = all {
        leader' = leader,
        replica_view' = replica_view,
    }

    action unchanged_all = all {
        msgs_signed_timeout' = msgs_signed_timeout, msgs_new_view' = msgs_new_view,
        msgs_proposal' = msgs_proposal, msgs_signed_commit' = msgs_signed_commit,
        store_signed_timeout' = store_signed_timeout, store_signed_commit' = store_signed_commit,
        proposer_view' = proposer_view, ghost_justifications' = ghost_justifications,
        leader' = leader,
        replica_view' = replica_view, replica_state' = replica_state,
        ghost_step' = ghost_step,
    }
}