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


    run weight_Test = all {
        TOTAL_WEIGHT == 6,
        QUORUM_WEIGHT == 5,
        SUBQUORUM_WEIGHT == 3,
        FAULTY_WEIGHT == 1
    }
    
    run get_implied_block_Test = {
        pure val commit_vote: CommitVote = {
            view: 0,
            block_number: 0,
            block_hash: hash("aa")
        }
        pure val commit: Justification = Commit({vote: commit_vote, agg_sig: Set("")})
    
        pure val timeout: Justification = Timeout({
            votes: Map(),
            agg_sig: Set(""),
            ghost_view: 1
        })

        all {
            get_implied_block(commit) == (1, None),
            get_implied_block(timeout) == (0, None),
        }
    }

    val vote1: CommitVote = {
        view: 1,
        block_number: 1,
        block_hash: "val_b0",
    }

    val vote2: CommitVote = {
        view: 1,
        block_number: 1,
        block_hash: "val_b1",
    }

    val vote3: CommitVote = {
        view: 2,
        block_number: 2,
        block_hash: "val_b2",
    }

    val commit_qc_1: CommitQC = {
        vote: vote1,
        agg_sig: Set("n0", "n1", "n2", "n3", "n5")
    } 

    val commit_qc_2: CommitQC = {
        vote: vote2,
        agg_sig: Set("n0", "n1", "n2", "n3", "n5")
    }

    val commit_qc_3: CommitQC = {
        vote: vote3,
        agg_sig: Set("n0", "n1", "n2", "n3", "n5")
    }

    val timeout_qc_1: TimeoutQC = {
        agg_sig: Set("n0", "n1", "n2", "n3", "n4", "n5"),
        ghost_view: 0,
        votes:
            Map(
                "n0" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 },
                "n1" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 },
                "n2" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 },
                "n3" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 },
                "n4" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 },
                "n5" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 }
            )
    }

    val timeout_qc_2: TimeoutQC = {
        agg_sig: Set("n0", "n1", "n2", "n3", "n4", "n5"),
        ghost_view: 0,
        votes:
            Map(
                "n0" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 0 },
                "n1" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 },
                "n2" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 },
                "n3" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 },
                "n4" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 },
                "n5" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 0 }
            )
    }

    val timeout_qc_3: TimeoutQC = {
        agg_sig: Set("n0", "n1", "n2", "n3", "n4", "n5"),
        ghost_view: 2,
        votes:
            Map(
                "n0" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n1" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n2" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n3" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n4" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n5" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 }
            )
    }

    val timeout_qc_4: TimeoutQC = {
        agg_sig: Set("n0", "n1", "n2", "n3", "n4", "n5"),
        ghost_view: 2,
        votes:
            Map(
                "n0" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n1" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n2" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n3" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n4" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote2), view: 2 },
                "n5" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote2), view: 2 }
            )
    }

    val timeout_qc_5: TimeoutQC = {
        agg_sig: Set("n0", "n1", "n2", "n3", "n4", "n5"),
        ghost_view: 2,
        votes:
            Map(
                "n0" -> { high_commit_qc: Some(commit_qc_1), high_vote: Some(vote1), view: 2 },
                "n1" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote1), view: 2 },
                "n2" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote2), view: 2 },
                "n3" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote2), view: 2 },
                "n4" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote3), view: 2 },
                "n5" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote3), view: 2 }
            )
    }

    val timeout_qc_6: TimeoutQC = {
        agg_sig: Set("n0", "n1", "n2", "n3", "n4", "n5"),
        ghost_view: 2,
        votes:
            Map(
                "n0" -> { high_commit_qc: Some(commit_qc_1), high_vote: Some(vote1), view: 2 },
                "n1" -> { high_commit_qc: Some(commit_qc_2), high_vote: Some(vote1), view: 2 },
                "n2" -> { high_commit_qc: Some(commit_qc_3), high_vote: Some(vote2), view: 2 },
                "n3" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote2), view: 2 },
                "n4" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote3), view: 2 },
                "n5" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote3), view: 2 }
            )
    }

    val timeout_qc_7: TimeoutQC = {
        agg_sig: Set("n0", "n1", "n2", "n3", "n4", "n5"),
        ghost_view: 2,
        votes:
            Map(
                "n0" -> { high_commit_qc: Some(commit_qc_1), high_vote: Some(vote1), view: 2 },
                "n1" -> { high_commit_qc: Some(commit_qc_2), high_vote: Some(vote1), view: 2 },
                "n2" -> { high_commit_qc: Some(commit_qc_3), high_vote: Some(vote1), view: 2 },
                "n3" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote2), view: 2 },
                "n4" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote2), view: 2 },
                "n5" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: Some(vote2), view: 2 }
            )
    }

    val timeout_qc_8: TimeoutQC = {
        agg_sig: Set("n0", "n1", "n2", "n3", "n4", "n5"),
        ghost_view: 2,
        votes:
            Map(
                "n0" -> { high_commit_qc: Some(commit_qc_1), high_vote: Some(vote1), view: 2 },
                "n1" -> { high_commit_qc: Some(commit_qc_1), high_vote: Some(vote1), view: 2 },
                "n2" -> { high_commit_qc: Some(commit_qc_1), high_vote: Some(vote1), view: 2 },
                "n3" -> { high_commit_qc: Some(commit_qc_1), high_vote: Some(vote1), view: 2 },
                "n4" -> { high_commit_qc: Some(commit_qc_1), high_vote: Some(vote1), view: 2 },
                "n5" -> { high_commit_qc: Some(commit_qc_1), high_vote: Some(vote1), view: 2 }
            )
    }

    run high_vote_Test = {

        all {
            assert(high_vote(timeout_qc_1) == None),
            assert(high_vote(timeout_qc_2) == None),
            assert(high_vote(timeout_qc_3) == Some(vote1)),
            assert(high_vote(timeout_qc_4) == Some(vote1)),
            assert(high_vote(timeout_qc_5) == None),
            assert(high_vote(timeout_qc_6) == None),
            assert(high_vote(timeout_qc_7) == None),
        }

    }

    run high_qc_Test = {
        all {
            assert(high_qc(timeout_qc_1) == HIGH_COMMIT_QC_NONE),
            assert(high_qc(timeout_qc_5) == Some(commit_qc_1)),
            assert(high_qc(timeout_qc_6) == Some(commit_qc_3)),
        }

    }

    run max_timeout_qc_Test = {
        all {
            max_timeout_qc(HIGH_TIMEOUT_QC_NONE, HIGH_TIMEOUT_QC_NONE) == HIGH_TIMEOUT_QC_NONE,
            max_timeout_qc(HIGH_TIMEOUT_QC_NONE, Some(timeout_qc_1)) == Some(timeout_qc_1),
            max_timeout_qc(Some(timeout_qc_1), HIGH_TIMEOUT_QC_NONE) == Some(timeout_qc_1),
            max_timeout_qc(Some(timeout_qc_7), Some(timeout_qc_1)) == Some(timeout_qc_7),
            max_timeout_qc(Some(timeout_qc_1), Some(timeout_qc_7)) == Some(timeout_qc_7),
        }
    }

    run max_commit_qc_Test = {
        all {
            max_commit_qc(HIGH_COMMIT_QC_NONE, HIGH_COMMIT_QC_NONE) == HIGH_COMMIT_QC_NONE,
            max_commit_qc(Some(commit_qc_1), HIGH_COMMIT_QC_NONE) == Some(commit_qc_1),
            max_commit_qc(HIGH_COMMIT_QC_NONE, Some(commit_qc_1)) == Some(commit_qc_1),
            max_commit_qc(Some(commit_qc_1), Some(commit_qc_3)) == Some(commit_qc_3),
            max_commit_qc(Some(commit_qc_3), Some(commit_qc_1)) == Some(commit_qc_3),
        }
    }

    val signed_timeout_votes_1: Set[SignedTimeoutVote] = Set(
        {
            vote: {
                view: 10,
                high_vote: COMMIT_VOTE_NONE,
                high_commit_qc: HIGH_COMMIT_QC_NONE
            },
            sig: "n1",
        }
    )   
     val signed_timeout_votes_2 = Set( 
        {
            vote: {
                view: 10,
                high_vote: COMMIT_VOTE_NONE,
                high_commit_qc: HIGH_COMMIT_QC_NONE
            },
            sig: "n2",
        },
        {
            vote: {
                view: 10,
                high_vote: COMMIT_VOTE_NONE,
                high_commit_qc: HIGH_COMMIT_QC_NONE
            },
            sig: "n3",
        },
        {
            vote: {
                view: 10,
                high_vote: COMMIT_VOTE_NONE,
                high_commit_qc: HIGH_COMMIT_QC_NONE
            },
            sig: "n4",
        },
        {
            vote: {
                view: 10,
                high_vote: COMMIT_VOTE_NONE,
                high_commit_qc: HIGH_COMMIT_QC_NONE
            },
            sig: "n1",
        },
        {
            vote: {
                view: 10,
                high_vote: COMMIT_VOTE_NONE,
                high_commit_qc: HIGH_COMMIT_QC_NONE
            },
            sig: "n0",
        }

    )

    run get_timeout_qc_Test = {
        val qc = {
                ghost_view: 10,
                agg_sig: Set("n0", "n1", "n2", "n3", "n4"),
                votes: 
                    Map(
                        "n0" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 10 },
                        "n1" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 10 },
                        "n2" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 10 },
                        "n3" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 10 },
                        "n4" -> { high_commit_qc: HIGH_COMMIT_QC_NONE, high_vote: COMMIT_VOTE_NONE, view: 10 }
                    )
            }
        all {
            get_timeout_qc(signed_timeout_votes_1, 10) == HIGH_TIMEOUT_QC_NONE,
            get_timeout_qc(signed_timeout_votes_2, 10) == Some(qc),
            get_timeout_qc(signed_timeout_votes_2, 100) == HIGH_TIMEOUT_QC_NONE
        }
    }
}