// -*- mode: Bluespec; -*-
// A restriction of replica.qnt to guide the model checker's search
module guided_replica {
    import types.* from "./types"
    import option.* from "./option"

    // the input trace to restrict the model checker's search space
    const G_INPUT_TRACE: List[StepKind]
    
    // the same parameters as in replica
    const G_WEIGHTS: ReplicaId -> Weight
    const G_REPLICA_KEYS: ReplicaId -> ReplicaKey
    const G_N: int
    const G_F: int
    const G_CORRECT: Set[ReplicaId]
    const G_FAULTY: Set[ReplicaId]
    const G_VIEWS: Set[ViewNumber]
    const G_VALID_BLOCKS: Set[Block]
    const G_INVALID_BLOCKS: Set[Block]

    import replica(
        WEIGHTS = G_WEIGHTS,
        REPLICA_KEYS = G_REPLICA_KEYS,
        N = G_N,
        F = G_F,
        CORRECT = G_CORRECT,
        FAULTY = G_FAULTY,
        VIEWS = G_VIEWS,
        VALID_BLOCKS = G_VALID_BLOCKS,
        INVALID_BLOCKS = G_INVALID_BLOCKS
    ).* from "./replica"

    // invariants
    val g_all_invariants = all_invariants
    val g_agreement_inv = agreement_inv
    val g_committed_blocks_have_justification_inv = committed_blocks_have_justification_inv
    val g_no_proposal_equivocation_inv = no_proposal_equivocation_inv
    val g_no_commit_equivocation_inv = no_commit_equivocation_inv
    val g_no_timeout_equivocation_inv = no_timeout_equivocation_inv
    val g_no_new_view_equivocation_inv = no_new_view_equivocation_inv
    val g_store_signed_commit_all_inv = store_signed_commit_all_inv
    val g_store_signed_timeout_all_inv = store_signed_timeout_all_inv
    val g_view_justification_inv = view_justification_inv
    val g_justification_is_supported_inv = justification_is_supported_inv
    val g_one_high_vote_in_timeout_qc_inv = one_high_vote_in_timeout_qc_inv
    val g_one_commit_quorum_inv = one_commit_quorum_inv
    val g_all_replicas_high_commit_qc_inv = all_replicas_high_commit_qc_inv
    val g_all_replicas_high_timeout_qc_inv = all_replicas_high_timeout_qc_inv
    val g_msgs_signed_timeout_inv = msgs_signed_timeout_inv
    val g_msgs_signed_commit_inv = msgs_signed_commit_inv
    val g_all_replicas_high_vote_inv = all_replicas_high_vote_inv
    val g_timeout_high_vote_is_highest_inv = timeout_high_vote_is_highest_inv

    // examples
    val g_two_chained_blocks_example = two_chained_blocks_example
    val g_one_block_example = one_block_example

    // remaining steps to replay
    var to_replay: List[StepKind]

    // initialize the replicas together with the trace to replay
    action g_init: bool = all {
        init,
        to_replay' = G_INPUT_TRACE,
    }

    // execute the same actions as in replica::step, but restrict their choice with to_replay
    action g_step = {
        any {
            // nothing to replay
            all {
                length(to_replay) == 0 or to_replay[0] == AnyStep,
                to_replay' = if (length(to_replay) == 0) to_replay else tail(to_replay),
                step,
            },
            // steps to replay
            all {
                length(to_replay) > 0,
                to_replay' = tail(to_replay),
                val expected_step = head(to_replay)
                any {
                    all {
                        expected_step == FaultyStep,         
                        faulty_step,
                        unchanged_replica,
                        leader' = leader,
                    },
                    nondet id = oneOf(CORRECT)
                    any {
                        all {
                            replica_view' = replica_view.set(id, replica_state.get(id).view),
                            on_timer_is_finished(id),
                            expected_step == OnTimerIsFinishedStep(id),
                            leader' = leader,
                        },
                        all {
                            replica_view' = replica_view.set(id, replica_state.get(id).view),
                            msgs_signed_commit != Set(),
                            nondet signed_vote = oneOf(msgs_signed_commit)
                            on_commit(id, signed_vote),
                            expected_step == OnCommitStep(id),
                            leader' = leader,
                        },
                        all {
                            replica_view' = replica_view.set(id, replica_state.get(id).view),
                            msgs_signed_timeout != Set(),
                            nondet vote = oneOf(msgs_signed_timeout)
                            on_timeout(id, vote),
                            expected_step == OnTimeoutStep(id),
                            leader' = leader,
                        },
                        all {
                            replica_view' = replica_view.set(id, replica_state.get(id).view),
                            msgs_proposal != Set(),
                            nondet proposal = oneOf(msgs_proposal)
                            on_proposal(id, proposal),
                            expected_step == OnProposalStep(id),
                            leader' = leader,
                        },
                        all {
                            replica_view' = replica_view.set(id, replica_state.get(id).view),
                            msgs_new_view != Set(),
                            nondet new_view = oneOf(msgs_new_view)
                            all {
                                on_new_view(id, new_view),
                                expected_step == OnNewViewStep({ id: id, view: new_view.justification.view() }),
                            },
                            leader' = leader,
                        },
                        all {
                                // Non-deterministically choose the next block, use it only for the case of None below.
                                nondet new_block = oneOf(VALID_BLOCKS)
                                proposer_step(id, new_block),
                                replica_view' = replica_view,
                                leader' = leader,
                                expected_step == ProposerStep(id),
                        },
                    },
                }
            }
        }
    }
}
