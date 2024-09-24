// -*- mode: Bluespec; -*-

/**
 * Replica in simplified ChonkyBFT.
 */
module replica {
    import types.* from "./types"
    import defs.* from "../defs"
    import option.* from "../option"

    // The number of all replicas.
    const N: int
    // Byzantine replica number threshold
    const F: int
    // Identities of the correct replicas.
    const CORRECT: Set[ReplicaId]
    // Identities of the faulty replicas.
    const FAULTY: Set[ReplicaId]
    // All replicas.
    pure val REPLICAS = CORRECT.union(FAULTY)
    // Check that threshold assumptions hold.
    assume FIVE_F = (N >= 5 * F + 1 and F >= 0)
    assume REPLICA_SIZE = (N == REPLICAS.size() and F == FAULTY.size())

    // The sum of all validator weights.
    pure val TOTAL_WEIGHT: Weight = N
    // The maximum weight of faulty replicas.
    // We want 5*FAULTY_WEIGHT + 1 = TOTAL_WEIGHT
    pure val FAULTY_WEIGHT: Weight = F
    // The weight threshold needed to form a quorum certificate.
    pure val QUORUM_WEIGHT: Weight = N - F
    // The weight threshold needed to trigger a reproposal.
    pure val SUBQUORUM_WEIGHT: Weight = N - 3 * F

    // The set of all views.
    const VIEWS: Set[ViewNumber]

    // the set of all blocks
    const VALID_BLOCKS: Set[Block]
    const INVALID_BLOCKS: Set[Block]
    pure val ALL_BLOCKS = VALID_BLOCKS.union(INVALID_BLOCKS)
    assume DISJOINT_BLOCKS = (VALID_BLOCKS.intersect(INVALID_BLOCKS) == Set())

    var replica_state: ReplicaId -> ReplicaState
    // the leader function from the view to the replicas identities
    var leader: ViewNumber -> ReplicaKey
    // an internal variable to handle timeouts
    var replica_view: ReplicaId -> ViewNumber
    // the current view of the proposer (only reading the replica state)
    var proposer_view: ReplicaId -> ViewNumber

    // the set of all Timeout messages sent in the past
    var msgs_signed_timeout: Set[SignedTimeoutVote]
    // the set of all SignedCommitVote messages sent in the past
    var msgs_signed_commit: Set[SignedCommitVote]
    // the set of all NewView messages sent in the past
    var msgs_new_view: Set[NewView]
    // the set of all Proposal messages sent in the past
    var msgs_proposal: Set[Proposal]

    // global storages of quorum certificates
    var ghost_timeout_qc: List[TimeoutQC]
    var ghost_commit_qc: List[CommitQC]

    // justifications stored by a replica
    var ghost_justifications: (ReplicaId, ViewNumber) -> Justification
    // save the last executed action
    var ghost_step: StepKind
    // We guess the sequence of blocks proposed by the correct leaders.
    // Since no correct leader would propose two different blocks at the same view,
    // it is sufficient to have this function.
    var prophecy_correct_proposals: ViewNumber -> Block

    val ALL_FAULTY_SIGNED_COMMIT_VOTES = 
        tuples(VIEWS, FAULTY, ALL_BLOCKS, VIEWS)
            .map( ((v, r, b, n)) => 
                {
                    vote:{
                        view: v, block_number: n, block_hash: b
                    },
                    sig: r
                })

    // This is a simple deterministic version that initializes
    // everything to the "genesis" state.
    // In general, we should start in an arbitrary replica_state that
    // satisfies certain conditions.
    // TODO: in the future, we will run the system in an intermediate state:
    // https://github.com/dnkolegov/chonkybft-spec/issues/23
    action init = {
        // non-deterministically choose the leader function
        nondet ldr = VIEWS.setOfMaps(REPLICAS).oneOf()
        init_view_1_with_leader(ldr)
    }

    action init_view_0_with_leader(ldr: ViewNumber -> ReplicaKey): bool = all {
        replica_state' = CORRECT.mapBy(id => {
            view: 0,
            high_vote: COMMIT_VOTE_NONE,
            high_commit_qc: HIGH_COMMIT_QC_NONE,
            high_timeout_qc: HIGH_TIMEOUT_QC_NONE,
            phase: PhaseTimeout,
            cached_proposals: Set(),
            committed_blocks: [],
        }),
        replica_view' = CORRECT.mapBy(id => 0),
        proposer_view' = CORRECT.mapBy(id => 0),
        // all correct replicas send SignedTimeoutVote as their first step
        msgs_signed_timeout' = CORRECT.map(id => {
            vote: { view: 0, high_vote: COMMIT_VOTE_NONE, high_commit_qc_view: None },
            high_commit_qc: HIGH_COMMIT_QC_NONE,
            sig: id
        }),
        msgs_signed_commit' = Set(),
        msgs_new_view' = Set(),
        msgs_proposal' = Set(),
        ghost_justifications' = Map(),
        leader' = ldr,
        ghost_commit_qc' = [],
        ghost_timeout_qc' = [],
        ghost_step' = InitStep,
        nondet i_prophecy_correct_proposals = VIEWS.setOfMaps(VALID_BLOCKS).oneOf()
        prophecy_correct_proposals' = i_prophecy_correct_proposals,
    }

    // start from a bootstrapped state in view 1
    action init_view_1_with_leader(ldr: ViewNumber -> ReplicaKey): bool = {
        // An initial quorum certificate.
        // We use all the correct + faulty replicas. Otherwise, when a quorum
        // of correct replicas is impossible, e.g., the actual number of faults is above F,
        // we cannot reach this state.
        pure val init_timeout_qc_id = 0
        pure val init_timeout_qc: TimeoutQC = {
            votes: REPLICAS.map(id => id).mapBy(_ => {
                view: 0,
                high_vote: COMMIT_VOTE_NONE,
                high_commit_qc_view: VIEW_NUMBER_NONE
            }),
            agg_sig: REPLICAS.map(id => id),
            high_commit_qc: HIGH_COMMIT_QC_NONE,
            ghost_view: 0,
        }
        pure val timeout_votes0 = REPLICAS.map(id => {
            sig: id,
            vote: { view: 0, high_vote: COMMIT_VOTE_NONE, high_commit_qc_view: VIEW_NUMBER_NONE },
            high_commit_qc: HIGH_COMMIT_QC_NONE,
        })
        all {
            replica_state' = CORRECT.mapBy(id => {
                view: 1,
                high_vote: COMMIT_VOTE_NONE,
                high_commit_qc: HIGH_COMMIT_QC_NONE,
                high_timeout_qc: Some(init_timeout_qc_id),
                phase: PhasePrepare,
                cached_proposals: Set(),
                committed_blocks: [],
            }),
            replica_view' = CORRECT.mapBy(id => 0),
            proposer_view' = CORRECT.mapBy(id => 0),
            // all replicas have sent TimeoutVote in view 0
            msgs_signed_timeout' = timeout_votes0,
            msgs_new_view' = CORRECT.map(id => {
                    { sig: id, justification: Timeout(init_timeout_qc_id) }
                }),
            msgs_signed_commit' = Set(),
            msgs_proposal' = Set(),
            // all correct correct replicas have Timeout(init_timeout_qc)
            ghost_justifications' =
                tuples(CORRECT, Set(1)).mapBy(((id, view)) => Timeout(init_timeout_qc_id)),
            leader' = ldr,
            ghost_commit_qc' = [],
            ghost_timeout_qc' = [init_timeout_qc],
            ghost_step' = InitStep,
            nondet i_prophecy_correct_proposals = VIEWS.setOfMaps(VALID_BLOCKS).oneOf()
            prophecy_correct_proposals' = i_prophecy_correct_proposals,
        }
    }

    // a single system step, including correct replicas & proposers, and faulty replicas & proposers
    action step = {
        any {
            correct_step,
            all {
                faulty_step,
                unchanged_replica,
                leader' = leader,
                prophecy_correct_proposals' = prophecy_correct_proposals,
            },
        }
    }

    // a single step + the fetcher algorithm
    action step_with_fetcher = {
        any {
            step,
            all {
                nondet id = oneOf(CORRECT)
                fetcher_step(id),
                leader' = leader,
                prophecy_correct_proposals' = prophecy_correct_proposals,
            },
        }
    }

    // same as step but correct replicas never timeout
    action step_no_timeout = {
        any {
            nondet id = oneOf(CORRECT)
            any {
                all {
                    replica_view' = replica_view.set(id, replica_state.get(id).view),
                    replica_step_no_timeout(id),
                    leader' = leader,
                    prophecy_correct_proposals' = prophecy_correct_proposals,
                },
                all {
                    // Non-deterministically choose the next block, use it only for the case of None below.
                    nondet new_block = oneOf(VALID_BLOCKS)
                    proposer_step(id, new_block),
                    replica_view' = replica_view,
                    leader' = leader,
                    prophecy_correct_proposals' = prophecy_correct_proposals,
                },
            },
            all {
                faulty_step,
                unchanged_replica,
                leader' = leader,
                prophecy_correct_proposals' = prophecy_correct_proposals,
            }
        }
    }

    // same as step but no faulty replicas (all FAULTY replicas are halted)
    action correct_step = {
        nondet id = oneOf(CORRECT)
        any {
            all {
                replica_view' = replica_view.set(id, replica_state.get(id).view),
                replica_step(id),
                leader' = leader,
                prophecy_correct_proposals' = prophecy_correct_proposals,
            },
            all {
                // Use the guessed block, use it only for the case of None below.
                val new_block = prophecy_correct_proposals.get(replica_state.get(id).view)
                proposer_step(id, new_block),
                replica_view' = replica_view,
                leader' = leader,
                prophecy_correct_proposals' = prophecy_correct_proposals,
            },
        }
    }
    
    // a single step by multiple Byzantine replicas
    action faulty_step: bool = all {
        FAULTY != Set(),
        any {
            all {
                nondet senders = FAULTY.powerset().oneOf()
                nondet commit_view = VIEWS.oneOf()
                nondet block_hash = ALL_BLOCKS.oneOf()
                nondet block_number = VIEWS.oneOf()
                val signed_commits = senders.map(s => {
                    vote:{
                        view: commit_view, block_number: block_number, block_hash: block_hash
                    },
                    sig: s
                })
                msgs_signed_commit' = msgs_signed_commit.union(signed_commits),
                msgs_signed_timeout' = msgs_signed_timeout,
                msgs_new_view' = msgs_new_view,
                msgs_proposal' = msgs_proposal,
                ghost_commit_qc' = ghost_commit_qc, ghost_timeout_qc' = ghost_timeout_qc,
            },
            all {
                // non-deterministically produce a timeout vote
                nondet msg_view = VIEWS.oneOf()
                nondet sig = oneOf(FAULTY)
                nondet commit_vote: SignedCommitVote = oneOf(msgs_signed_commit.union(ALL_FAULTY_SIGNED_COMMIT_VOTES))
                nondet commit_qc_view = oneOf(VIEWS)
                nondet commit_qc_block_number = oneOf(VIEWS)
                nondet commit_qc_block_hash = oneOf(ALL_BLOCKS)
                nondet commit_qc_agg_sig = REPLICAS.powerset().oneOf()
                val commit_qc_vote: CommitVote = {
                    view: commit_qc_view,
                    block_number: commit_qc_block_number,
                    block_hash: commit_qc_block_hash
                }
                val commit_qc = {
                    vote: commit_qc_vote,
                    agg_sig: commit_qc_agg_sig
                }
                val commit_qc_id = ghost_commit_qc.length()
                nondet has_high_vote = oneOf(Bool)
                nondet has_commit_qc = oneOf(Bool)
                nondet high_commit_qc_view = oneOf(VIEWS)
                nondet has_high_commit_qc_view = oneOf(Bool)
                val signed_timeout_vote = {
                    vote: {
                        view: msg_view,
                        high_vote: if (has_high_vote) Some(commit_vote.vote) else COMMIT_VOTE_NONE,
                        high_commit_qc_view: if (has_high_commit_qc_view) Some(high_commit_qc_view) else VIEW_NUMBER_NONE,
                    },
                    high_commit_qc: if (has_commit_qc) Some(commit_qc_id) else HIGH_COMMIT_QC_NONE,
                    sig: sig,
                }
                all {
                    // faulty replicas cannot forge the signatures of the correct replicas, enforce that
                    commit_qc_agg_sig.forall(id => or {
                        FAULTY.contains(id),
                        msgs_signed_commit.exists(sv => sv.sig == id and sv.vote == commit_qc_vote),
                    }),
                    // add the message by a faulty replica
                    msgs_signed_timeout' = msgs_signed_timeout.union(Set(signed_timeout_vote)),
                    msgs_signed_commit' = msgs_signed_commit,
                    msgs_new_view' = msgs_new_view,
                    msgs_proposal'  = msgs_proposal,
                    ghost_commit_qc' = ghost_commit_qc.append(commit_qc),
                    ghost_timeout_qc' = ghost_timeout_qc,
                }
            },
            all {
                nondet msg_view = VIEWS.oneOf()
                nondet sig = oneOf(FAULTY)
                nondet justification_case = Set("commit", "timeout").oneOf()
                // introduce a commit qc
                nondet commit_qc_view = oneOf(VIEWS)
                nondet commit_qc_block_number = oneOf(VIEWS)
                nondet commit_qc_block_hash = oneOf(ALL_BLOCKS)
                nondet commit_qc_agg_sig = REPLICAS.powerset().oneOf()
                val commit_qc_vote: CommitVote = {
                    view: commit_qc_view,
                    block_number: commit_qc_block_number,
                    block_hash: commit_qc_block_hash
                }
                val commit_qc = {
                    vote: commit_qc_vote, agg_sig: commit_qc_agg_sig,
                }
                val commit_qc_id = ghost_commit_qc.length()
                // Introduce a timeout qc.
                // We recycle commit_qc when constructing timeout_qc, as commit_qc and timeout_qc
                // are not used at the same time, see justification_case.
                nondet timeout_qc_view = VIEWS.oneOf()
                nondet timeout_qc_voters = REPLICAS.powerset().oneOf()
                nondet timeout_qc_agg_sig = timeout_qc_voters.powerset().oneOf()
                nondet timeout_qc_votes =
                    timeout_qc_voters.setOfMaps(msgs_signed_timeout.map(sv => sv.vote)).oneOf()
                nondet has_high_commit_qc_view = oneOf(Bool)
                val timeout_qc: TimeoutQC = {
                    votes: timeout_qc_votes,
                    agg_sig: timeout_qc_agg_sig,
                    high_commit_qc:
                        if (has_high_commit_qc_view) Some(commit_qc_id) else HIGH_COMMIT_QC_NONE,
                    ghost_view: timeout_qc_view,
                }
                val timeout_qc_id = ghost_timeout_qc.length()
                // make new view
                val new_view: NewView = {
                    justification:
                        if (justification_case == "commit") {
                            Commit(commit_qc_id)
                        } else {
                            Timeout(timeout_qc_id)
                        },
                    sig: sig,
                }
                all {
                    // faulty replicas cannot forge the signatures of the correct replicas, enforce that
                    commit_qc_agg_sig.forall(id => or {
                        FAULTY.contains(id),
                        msgs_signed_commit.exists(sv => sv.sig == id and sv.vote == commit_qc_vote),
                    }),
                    justification_case != "commit" implies timeout_qc_agg_sig.forall(id => or {
                        FAULTY.contains(id),
                        msgs_signed_timeout.exists(sv => sv.sig == id and sv.vote == timeout_qc_votes.get(id)),
                    }),
                    // send a new view
                    msgs_new_view' = msgs_new_view.union(Set(new_view)),
                    msgs_signed_commit' = msgs_signed_commit,
                    msgs_signed_timeout' = msgs_signed_timeout,
                    msgs_proposal' = msgs_proposal,
                    ghost_commit_qc' = ghost_commit_qc.append(commit_qc),
                    ghost_timeout_qc' = ghost_timeout_qc.append(timeout_qc),
                }
            },
            all {
                nondet msg_view = VIEWS.oneOf()
                nondet sig = oneOf(FAULTY)
                nondet justification_case = Set("commit", "timeout").oneOf()
                // introduce a commit qc
                nondet commit_qc_view = oneOf(VIEWS)
                nondet commit_qc_block_number = oneOf(VIEWS)
                nondet commit_qc_block_hash = oneOf(ALL_BLOCKS)
                nondet commit_qc_agg_sig = REPLICAS.powerset().oneOf()
                val commit_qc_vote: CommitVote = {
                    view: commit_qc_view, block_number: commit_qc_block_number,
                    block_hash: commit_qc_block_hash
                }
                val commit_qc = {
                    vote: commit_qc_vote, agg_sig: commit_qc_agg_sig,
                }
                val commit_qc_id = ghost_commit_qc.length()
                // Introduce a timeout qc.
                // We recycle commit_qc when constructing timeout_qc, as commit_qc and timeout_qc
                // are not used at the same time, see justification_case.
                nondet timeout_qc_view = VIEWS.oneOf()
                nondet timeout_qc_voters = REPLICAS.powerset().oneOf()
                nondet timeout_qc_agg_sig = timeout_qc_voters.powerset().oneOf()
                nondet timeout_qc_votes =
                    timeout_qc_voters.setOfMaps(msgs_signed_timeout.map(sv => sv.vote)).oneOf()
                nondet has_high_commit_qc_view = oneOf(Bool)
                val timeout_qc: TimeoutQC = {
                    votes: timeout_qc_votes,
                    agg_sig: timeout_qc_agg_sig,
                    high_commit_qc: if (has_high_commit_qc_view) Some(commit_qc_id) else HIGH_COMMIT_QC_NONE,
                    ghost_view: timeout_qc_view,
                }
                val timeout_qc_id = ghost_timeout_qc.length()
                // non-deterministically select a block
                nondet block = ALL_BLOCKS.oneOf()
                nondet block_number = VIEWS.oneOf()
                nondet has_block = oneOf(Bool)
                // make new proposal
                val proposal: Proposal = {
                    block: if (has_block) Some(block) else None,
                    justification:
                        if (justification_case == "commit") {
                            Commit(commit_qc_id)
                        } else {
                            Timeout(timeout_qc_id)
                        },
                    sig: sig,
                    ghost_block_number: block_number,
                }
                all {
                    // faulty replicas cannot forge the signatures of the correct replicas, enforce that
                    commit_qc_agg_sig.forall(id => or {
                        FAULTY.contains(id),
                        msgs_signed_commit.exists(sv => sv.sig == id and sv.vote == commit_qc_vote),
                    }),
                    justification_case != "commit" implies timeout_qc_agg_sig.forall(id => or {
                        FAULTY.contains(id),
                        msgs_signed_timeout.exists(sv => sv.sig == id and sv.vote == timeout_qc_votes.get(id)),
                    }),
                    msgs_proposal' = msgs_proposal.union(Set(proposal)),
                    msgs_signed_commit' = msgs_signed_commit,
                    msgs_signed_timeout' = msgs_signed_timeout,
                    msgs_new_view' = msgs_new_view,
                    ghost_commit_qc' = ghost_commit_qc.append(commit_qc),
                    ghost_timeout_qc' = ghost_timeout_qc.append(timeout_qc),
                },
            },
        },
        ghost_step' = FaultyStep,
    }

    // a step by one of the replicas without timeouts
    action replica_step_no_timeout(id: ReplicaId):bool = all{
        any {
            nondet signed_votes = msgs_signed_commit.powerset().oneOf()
            all {
                signed_votes != Set(),
                on_commit(id, signed_votes),
            },
            nondet signed_votes = msgs_signed_timeout.powerset().oneOf()
            all {
                signed_votes != Set(),
                on_timeout(id, signed_votes),
            },
            all {
                msgs_proposal != Set(),
                nondet proposal = oneOf(msgs_proposal)
                on_proposal(id, proposal),
            },
            all {
                msgs_new_view != Set(),
                nondet new_view = oneOf(msgs_new_view)
                on_new_view(id, new_view),
            }
        },
    }

    // a step by one of the correct replicas
    action replica_step(id: ReplicaId):bool = all{
        any {
            on_timer_is_finished(id),
            replica_step_no_timeout(id),
        },
    }

    action on_proposal(id: ReplicaId, proposal: Proposal): bool = all {
        val self = replica_state.get(id)
        // Get the proposal view.
        val proposal_view = proposal.justification.view()
        all {
            // We only allow proposals for the current view if we have not voted in it yet.
            or {
                proposal_view == self.view and self.phase == PhasePrepare,
                proposal_view > self.view
            },
            // since our views are bounded, we ignores out of bounds views
            VIEWS.contains(proposal_view),
            // We ignore proposals from the wrong leader.
            proposal.sig == leader.get(proposal_view),
            // Check that the proposal is valid.
            proposal.proposal_verify(),
            // Get the implied block number and hash (if any).
            val block_number_and_opt_block_hash = proposal.justification.get_implied_block()
            val block_number = block_number_and_opt_block_hash._1
            val opt_block_hash = block_number_and_opt_block_hash._2
            // Vote only if you have collected all committed blocks so far.
            // (See a check in all {...} below.)

            // Check if this is a reproposal or not, and do the necessary checks.
            // As a side result, get the correct block hash.
            val block_hash: BlockHash = match(opt_block_hash) {
                | Some(hash) => {
                    // This is a reproposal. We let the leader repropose blocks without sending
                    // them in the proposal (it sends only the number + hash). That allows a
                    // leader to repropose a block without having it stored.
                    // It is an optimization that allows us to not wait for a leader that has
                    // the previous proposal stored (which can take 4f views), and to somewhat
                    // speed up reproposals by skipping block broadcast.
                    // This only saves time because we have a gossip network running in parallel,
                    // and any time a replica is able to create a finalized block (by possessing
                    // both the block and the commit QC) it broadcasts the finalized block (this
                    // was meant to propagate the block to full nodes, but of course validators
                    // will end up receiving it as well).
                    // However, this can be difficult to model and we might want to just
                    // ignore the gossip network in the formal model. We will still have liveness
                    // but in the model we'll end up waiting 4f views to get a leader that has the
                    // previous block before proposing a new one. This is not that bad, since
                    // then we can be sure that the consensus will continue even if the gossip
                    // network is failing for some reason.

                    // See validation below.
                    hash
                }

                | None => {
                    // See state verification below.
                    unwrap_or(proposal.block, "").hash()
                }
            }

            // the replica vote to send
            val vote: CommitVote = {
                view: proposal_view,
                block_number: block_number,
                block_hash: block_hash,
            }

            all {
                // Vote only if you have collected all committed blocks so far (see a check in all {...}).
                block_number == length(self.committed_blocks),
                // Validation.
                if (is_some(opt_block_hash)) {
                    // For sanity reasons, we'll check that there's no block in the proposal.
                    // But this check should be completely unnecessary (in theory at least).
                    is_none(proposal.block)
                } else {
                    all {
                        // This is a new proposal, so we need to verify it (i.e. execute it).
                        is_some(proposal.block),
                        // To verify the block, replica just tries to apply it to the current
                        // state. Current state is the result of applying all the committed blocks until now.
                        proposal.block.block_verify(block_number)
                        // We cache the new proposals, waiting for them to be committed.
                        // See the update to cached_proposals below.
                    }
                },
                // Update the replica state
                val self1 = {
                    ...self,
                    view: proposal_view,
                    phase: PhaseCommit,
                    high_vote: Some(vote),
                    cached_proposals:
                        if (is_none(proposal.block)) {
                            self.cached_proposals
                        } else {
                            // the implementation caches the block, we simply mark its number + hash
                            self.cached_proposals.union(Set((block_number, block_hash)))
                        }
                }
                val self2 =
                    match (proposal.justification) {
                    | Commit(qc_id) =>
                        self1.process_commit_qc(Some(qc_id))

                    | Timeout(qc_id) =>
                        val qc = ghost_timeout_qc[qc_id]
                        {
                            ...self1.process_commit_qc(qc.high_commit_qc),
                            high_timeout_qc: max_timeout_qc(Some(qc_id), self1.high_timeout_qc)
                        }
                    }

                replica_state' = replica_state.set(id, self2),
                // Send the commit vote to all replicas (including ourselves).
                msgs_signed_commit' = msgs_signed_commit.union(Set({ vote: vote, sig: id })),
                // Record the justification.
                ghost_justifications' =
                    ghost_justifications.put((id, proposal.justification.view()), proposal.justification),
                // unchanged variables
                msgs_signed_timeout' = msgs_signed_timeout, msgs_new_view' = msgs_new_view,
                msgs_proposal' = msgs_proposal,
                proposer_view' = proposer_view,
                ghost_commit_qc' = ghost_commit_qc,
                ghost_timeout_qc' = ghost_timeout_qc,
                ghost_step' = OnProposalStep(id),
            }
        }
    }

    // a replica receives SignedCommitVote
    action on_commit(id: ReplicaId, signed_votes: Set[SignedCommitVote]): bool = all {
        val self = replica_state.get(id)
        nondet base_vote = signed_votes.oneOf()
        all {
            // if the vote isn't current, just ignore it.
            base_vote.vote.view >= self.view,
            signed_votes.forall(v => v.vote.view == base_vote.vote.view),
            // Check that the signed vote is valid.
            signed_votes.forall(v => signed_commit_vote_verify(v)),

            // We will never store duplicate (same view and sender) votes.
            signed_votes.forall(v1 => signed_votes.forall(v2 => v1.sig != v2.sig or v1.vote == v2.vote)),

            all {
                // Check if we now have a commit QC for this view.
                val qc_opt = get_commit_qc(signed_votes, base_vote)
                match (qc_opt) {
                    | None => all {
                        replica_state' = replica_state,
                        msgs_new_view' = msgs_new_view,
                        ghost_commit_qc' = ghost_commit_qc,
                        ghost_justifications' = ghost_justifications,
                    }

                    | Some(qc) => {
                        val qc_id = ghost_commit_qc.length()
                        val self1 = self.process_commit_qc(Some(qc_id))
                        all {
                            ghost_commit_qc' = ghost_commit_qc.append(qc),
                            start_new_view(id, self1, base_vote.vote.view + 1, Commit(qc_id))
                        }
                    }
                },

                msgs_signed_commit' = msgs_signed_commit,
                msgs_proposal' = msgs_proposal, msgs_signed_timeout' = msgs_signed_timeout,
                proposer_view' = proposer_view,
                ghost_timeout_qc' = ghost_timeout_qc,
                ghost_step' = OnCommitStep(id),
            }
        }
    }

    // If this is the first view, we immediately timeout. This will force the replicas
    // to synchronize right at the beginning and will provide a justification for the
    // proposer at view 1.
    // Currently, this is completely non-deterministic, that is,
    // this action can be triggered at any step
    // TODO: currently, timers are non-deterministic, model them as integers?
    // https://github.com/dnkolegov/chonkybft-spec/issues/25
    action on_timer_is_finished(id: ReplicaId): bool = {
        val self = replica_state.get(id)
        val vote: SignedTimeoutVote = {
            vote: {
                view: self.view,
                high_vote: self.high_vote,
                high_commit_qc_view:
                    match (self.high_commit_qc) {
                    | None => VIEW_NUMBER_NONE
                    | Some(qc_id) =>
                        val qc = ghost_commit_qc[qc_id]
                        Some(qc.vote.view)
                    }
            },
            sig: id,
            high_commit_qc: self.high_commit_qc,
        }
        all {
            // If we have already timed out, we don't need to send another timeout vote.
            self.phase != PhaseTimeout,
            replica_state' = replica_state.set(id, { ...self, phase: PhaseTimeout }),
            msgs_signed_timeout' = msgs_signed_timeout.union(Set(vote)),
            // unchanged variables
            msgs_signed_commit' = msgs_signed_commit, msgs_new_view' = msgs_new_view,
            msgs_proposal' = msgs_proposal,
            proposer_view' = proposer_view, ghost_justifications' = ghost_justifications,
            ghost_timeout_qc' = ghost_timeout_qc,
            ghost_commit_qc' = ghost_commit_qc,
            ghost_step' = OnTimerIsFinishedStep(id),
        }
    }

    // a replica receives SignedTimeoutVote
    action on_timeout(id: ReplicaId, signed_votes: Set[SignedTimeoutVote]): bool = all {
        nondet common_view = VIEWS.oneOf()
        val self = replica_state.get(id)
        all {
            // If the vote isn't current, just ignore it
            common_view >= self.view,
            signed_votes.forall(v => v.vote.view == common_view),
            // Check that the signed vote is valid.
            signed_votes.forall(v => signed_timeout_vote_verify(v)),
            // We will never store duplicate (same view and sender) votes.
            signed_votes.forall(v1 => signed_votes.forall(v2 => v1.sig != v2.sig or v1.vote == v2.vote)),

            all {
                // Check if we now have a timeout QC for this view.
                val qc_opt = get_timeout_qc(signed_votes, common_view)
                match (qc_opt) {
                    | None => all {
                        replica_state' = replica_state,
                        msgs_new_view' = msgs_new_view,
                        ghost_timeout_qc' = ghost_timeout_qc,
                        ghost_justifications' = ghost_justifications,
                    }

                    | Some(qc) => all {
                        ghost_timeout_qc' = ghost_timeout_qc.append(qc),
                        val qc_id = ghost_timeout_qc.length()
                        val self1 = {
                            ...self.process_commit_qc(qc.high_commit_qc),
                            high_timeout_qc: max_timeout_qc(Some(qc_id), self.high_timeout_qc)
                        }
                        start_new_view(id, self1, common_view + 1, Timeout(qc_id)),
                    }
                },
                // unchanged variables
                msgs_signed_timeout' = msgs_signed_timeout, msgs_signed_commit' = msgs_signed_commit,
                msgs_proposal' = msgs_proposal, proposer_view' = proposer_view,
                ghost_commit_qc' = ghost_commit_qc,
                ghost_step' = OnTimeoutStep(id),
            }
        }
    }

    // a replica receives NewView
    action on_new_view(id: ReplicaId, new_view: NewView): bool = all {
        val self = replica_state.get(id)
        val new_view_view = new_view.justification.view()
        all {
            // If the message isn't current, just ignore it.
            new_view_view >= self.view,
            // Check that the new view is valid.
            new_view.new_view_verify(),
            // Update our state.
            val self1 =
                match (new_view.justification) {
                    | Commit(qc_id) => self.process_commit_qc(Some(qc_id))

                    | Timeout(qc_id) =>
                        val qc = ghost_timeout_qc[qc_id]
                        {
                            ...self.process_commit_qc(qc.high_commit_qc),
                            high_timeout_qc: max_timeout_qc(Some(qc_id), self.high_timeout_qc)
                        }
                }

            if (new_view_view > self.view) {
                start_new_view(id, self1, new_view_view, new_view.justification)
            } else {
                all {
                    replica_state' = replica_state,
                    msgs_new_view' = msgs_new_view,
                    ghost_justifications' = ghost_justifications,
                }
            },
            // unchanged variables
            msgs_signed_timeout' = msgs_signed_timeout, msgs_signed_commit' = msgs_signed_commit,
            msgs_proposal' = msgs_proposal,
            proposer_view' = proposer_view,
            ghost_commit_qc' = ghost_commit_qc,
            ghost_timeout_qc' = ghost_timeout_qc,
            ghost_step' = OnNewViewStep({ id: id, view: new_view_view }),
        }
    }

    // A step by the proposer for replica_id.
    // We pass new_block, which is used a substitute for: self.create_proposal(block_number).
    action proposer_step(replica_id: ReplicaId, new_block: Block): bool = all {
        val replica_state_view = replica_state.get(replica_id).view
        val proposer_state_view = proposer_view.get(replica_id)
        all {
            // Check if we are in a new view and we are the leader.
            // Note that by definition, it's impossible to be the leader for view 0.
            // We always let view 0 timeout.
            // This code only works for a non-faulty proposer.
            CORRECT.contains(replica_id),
            proposer_state_view < replica_state_view,
            replica_id == leader.get(replica_state_view),
            proposer_view' = proposer_view.set(replica_id, replica_state_view),
            // Get the justification for this view. If we have both a commit QC
            // and a timeout QC for this view (highly unlikely), we should prefer
            // to use the commit QC.
            val justification = ghost_justifications.get((replica_id, replica_state_view))
            // Get the block number and check if this must be a reproposal.
            val block_and_hash = justification.get_implied_block()
            val block_number = block_and_hash._1
            val opt_block_hash = block_and_hash._2
            // Propose only if you have collected all committed blocks so far: see in all {...}
            // Now we create the block.
            val block =
                match (opt_block_hash) {
                | Some(h) =>
                    // There was some proposal last view that at least SUBQUORUM_WEIGHT replicas
                    // voted for and could have been finalized. We need to repropose it.
                    None
                | None =>
                    // The previous proposal was finalized, so we can propose a new block.
                    // If we don't have the previous block, this call will fail. This is
                    // fine as we can just not propose anything and let our turn end.
                    // (see the check below)
                    Some(new_block)
                }

            // Create the proposal message and send it to all replicas
            val proposal: Proposal = {
                block: block,
                justification: justification,
                sig: replica_id,
                ghost_block_number: block_number,
            }
            all {
                // Propose only if you have collected all committed blocks so far.
                val committed_blocks = replica_state.get(replica_id).committed_blocks
                block_number == length(committed_blocks),
                // check that we have received the previous block, if we are proposing a new one
                or {
                    is_none(block),
                    block_number == 0,
                    replica_state.get(replica_id).cached_proposals.exists(((bn, bh)) => {
                        block_number == bn + 1
                    }),
                },
                // send the proposal
                msgs_proposal' = msgs_proposal.union(Set(proposal)),
            },
            // unchanged variables
            replica_state' = replica_state, msgs_signed_timeout' = msgs_signed_timeout,
            msgs_signed_commit' = msgs_signed_commit, msgs_new_view' = msgs_new_view,
            ghost_justifications' = ghost_justifications,
            ghost_commit_qc' = ghost_commit_qc, ghost_timeout_qc' = ghost_timeout_qc,
            ghost_step' = ProposerStep(replica_id),
        }
    }

    // a correct replica changes to a new view
    action start_new_view(id: ReplicaId, self: ReplicaState, view: ViewNumber, justification: Justification): bool = {
        // make sure that we do not go out of bounds with the new view
        all {
            VIEWS.contains(view),
            // switch to the Prepare phase
            replica_state' = replica_state.set(id, { ...self, view: view, phase: PhasePrepare }),
            // Send a new view message to the other replicas for synchronization
            msgs_new_view' = msgs_new_view.union(Set({ justification: justification, sig: id })),
            // store the justification
            ghost_justifications' = ghost_justifications.put((id, view), justification)
        }
    }

    // fetch a missing block from the peers (executed concurrently with the proposer and replica)
    action fetcher_step(id: ReplicaId): bool = {
        val self = replica_state.get(id)
        match (self.high_commit_qc) {
        | None =>
            all {
                // do nothing
                false,
                unchanged_replica,
                msgs_signed_timeout' = msgs_signed_timeout, msgs_new_view' = msgs_new_view,
                msgs_proposal' = msgs_proposal, msgs_signed_commit' = msgs_signed_commit,
                ghost_commit_qc' = ghost_commit_qc, ghost_timeout_qc' = ghost_timeout_qc,
                ghost_step' = ghost_step,
            }

        | Some(qc_id) =>
            val next_block_number = self.committed_blocks.length()
            val qc = ghost_commit_qc[qc_id]
            all {
                qc.vote.block_number >= next_block_number,
                // query other replicas whether they have a CommittedBlock
                // with the given number, and fetch it from them. The fetched block and
                // its commit_qc are verified locally.
                any {
                    // fetch the block from a correct replica
                    nondet other_id = oneOf(CORRECT)
                    val other = replica_state.get(other_id)
                    all {
                        other.committed_blocks.length() > next_block_number,
                        val fetched_block = other.committed_blocks[next_block_number]
                        val new_self = {
                            ...self,
                            committed_blocks: self.committed_blocks.append(fetched_block)
                        }
                        replica_state' = replica_state.set(id, new_self),
                        ghost_commit_qc' = ghost_commit_qc,
                    },
                    // fetch the block from a faulty replica
                    // introduce a commit qc
                    nondet commit_qc_view = oneOf(VIEWS)
                    nondet commit_qc_block_number = oneOf(VIEWS)
                    nondet commit_qc_block = oneOf(ALL_BLOCKS)
                    nondet commit_qc_agg_sig = REPLICAS.powerset().oneOf()
                    val commit_qc_vote: CommitVote = {
                        view: commit_qc_view, block_number: commit_qc_block_number,
                        block_hash: commit_qc_block
                    }
                    val commit_qc = { vote: commit_qc_vote, agg_sig: commit_qc_agg_sig }
                    val commit_qc_id = length(ghost_commit_qc)
                    val fetched_block: CommittedBlock = { commit_qc: commit_qc_id, block: commit_qc_block }
                    // verify the block
                    all {
                        FAULTY != Set(),
                        // faulty replicas cannot forge the signatures of the correct replicas, enforce that
                        commit_qc_agg_sig.forall(id => or {
                            FAULTY.contains(id),
                            msgs_signed_commit.exists(sv => sv.sig == id and sv.vote == commit_qc_vote),
                        }),
                        // the replica verifies the received commit_qc
                        commit_qc_verify(commit_qc),
                        // store the received block
                        val new_self = {
                            ...self,
                            committed_blocks: self.committed_blocks.append(fetched_block)
                        }
                        replica_state' = replica_state.set(id, new_self),
                        ghost_commit_qc' = ghost_commit_qc.append(commit_qc),
                    }
                },
                ghost_step' = FetchStep(id),
                proposer_view' = proposer_view, ghost_justifications' = ghost_justifications, replica_view' = replica_view,
                msgs_signed_timeout' = msgs_signed_timeout, msgs_new_view' = msgs_new_view,
                msgs_proposal' = msgs_proposal, msgs_signed_commit' = msgs_signed_commit,
                ghost_timeout_qc' = ghost_timeout_qc,
            }
        }
    }

    // part of the state that is not touched by the Byzantine validators
    action unchanged_replica = all {
        proposer_view' = proposer_view, ghost_justifications' = ghost_justifications,
        replica_view' = replica_view, replica_state' = replica_state,
    }

    // -------------------------------------------------------------------------
    // Auxiliary definitions

    def block_verify(block: Option[Block], implied_block_number: BlockNumber): bool = {
        // Validation is application-specific.
        // For specification purposes, we check validity via ValidBlocks.
        match (block) {
            | Some(blk) => and {
                    VALID_BLOCKS.contains(blk),
                    // The implementation should check that the implied block number
                    // belongs to the data blob passed in block, as it can interpret the block.
                    // The specification does not make such a connection.
                    // The relation between the block numbers and committed blocks
                    // is checked in on_proposal.
                    implied_block_number >= 0,
                }
            | None => false
        }
    }

    // Processed an (already verified) commit_qc received from the network
    // as part of some message. It bumps the local high_commit_qc and if
    // we have the proposal corresponding to this qc, we append it to the committed_blocks.
    def process_commit_qc(self: ReplicaState, qc_opt: Option[QcId]): ReplicaState = {
        match (qc_opt) {
            | None => self

            | Some(qc_id) =>
                val qc = ghost_commit_qc[qc_id]
                if (self.cached_proposals.contains((qc.vote.block_number, qc.vote.block_hash))
                        and self.committed_blocks.length() == qc.vote.block_number) {
                    // NOTE: the implementation retrieves the block by its hash from cached_proposals.
                    // In the Quint spec, the hash and the block are identical, so we simply put the hash.
                    val block = qc.vote.block_hash
                    {
                        ...self,
                        committed_blocks: self.committed_blocks.append({ block: block, commit_qc: qc_id }),
                        high_commit_qc: max_commit_qc(Some(qc_id), self.high_commit_qc)
                    }
                } else {
                    {
                        ...self,
                        high_commit_qc: max_commit_qc(Some(qc_id), self.high_commit_qc)
                    }
                }
        }
    }

    // given two QCs, return the one with the largest view,
    // or the left one, in case of the same view
    def max_timeout_qc(qc1_opt: Option[QcId], qc2_opt: Option[QcId]): Option[QcId] = {
        match (qc2_opt) {
            | None => qc1_opt
            | Some(qc2_id) =>
                match (qc1_opt) {
                    | None => qc2_opt
                    | Some(qc1_id) =>
                        val qc1 = ghost_timeout_qc[qc1_id]
                        val qc2 = ghost_timeout_qc[qc2_id]
                        if (qc1.ghost_view >= qc2.ghost_view) qc1_opt else qc2_opt
                }
        }
    }

    // given two QCs, return the one with the largest view,
    // or the left one, in case of the same view
    def max_commit_qc(qc1_opt: Option[QcId], qc2_opt: Option[QcId]): Option[QcId] = {
        match (qc2_opt) {
            | None => qc1_opt
            | Some(qc2_id) =>
                match(qc1_opt) {
                    | None => qc2_opt
                    | Some(qc1_id) =>
                        val qc1 = ghost_commit_qc[qc1_id]
                        val qc2 = ghost_commit_qc[qc2_id]
                        if (qc1.vote.view >= qc2.vote.view) qc1_opt else qc2_opt
                }
        }
    }

    // If there is a quorum among timeout votes, return `Some(qc)`,
    // where qc is the largest quorum among the votes. Otherwise, return None.
    def get_timeout_qc(votes: Set[SignedTimeoutVote], view: ViewNumber): Option[TimeoutQC] = {
        val view_signed_votes = votes.filter(v => v.vote.view == view)
        if (view_signed_votes.size() < QUORUM_WEIGHT) {
            None
        } else {
            val high_commit_qc: Option[QcId] = view_signed_votes.fold(HIGH_COMMIT_QC_NONE,
                (prev_max_qc, sv) => max_commit_qc(prev_max_qc, sv.high_commit_qc))
            // Importantly, view_signed_votes does not have two votes with the same signature.
            // We check that in store_signed_timeout_all_inv.
            val qc_votes: ReplicaKey -> TimeoutVote =
                view_signed_votes.map(v => (v.sig, v.vote)).setToMap()
            Some({
                votes: qc_votes,
                agg_sig: view_signed_votes.map(v => v.sig),
                high_commit_qc: high_commit_qc,
                ghost_view: view,
            })
        }
    }

    // If we have identical commit votes with at least QUORUM_WEIGHT
    // from different replicas, we can create a commit quorum certificate locally, and return `Some(qc)`.
    // Otherwise, return None.
    def get_commit_qc(votes: Set[SignedCommitVote], last_signed_vote: SignedCommitVote): Option[CommitQC] = {
        // group the votes by the same view, same block hash, and same block number
        pure val similar_votes: Set[SignedCommitVote] =
            votes.filter(v => and {
                v.vote.view == last_signed_vote.vote.view,
                v.vote.block_hash == last_signed_vote.vote.block_hash,
                v.vote.block_number == last_signed_vote.vote.block_number,
            })
        // count the votes for the block hash of vote
        pure val votes_count = similar_votes.size()
        if (votes_count < QUORUM_WEIGHT) {
            None
        } else {
            // There is only one block hash that has a quorum.
            // If this is not the case, one_commit_quorum is violated.
            Some({
                vote: last_signed_vote.vote,
                agg_sig: similar_votes.map(v => v.sig),
            })
        }
    }

    def justification_verify(justification: Justification): bool = {
        match (justification) {
            | Timeout(qc_id) =>
                val qc = ghost_timeout_qc[qc_id]
                qc.timeout_qc_verify()

            | Commit(qc_id) =>
                val qc = ghost_commit_qc[qc_id]
                qc.commit_qc_verify()
        }
    }

    def proposal_verify(p: Proposal):bool = all {
        p.justification.justification_verify(),
    }

    // Get the view of a justification
    def view(justification: Justification): ViewNumber = {
        match (justification) {
            | Commit(qc_id) => ghost_commit_qc[qc_id].vote.view + 1
            | Timeout(qc_id) => ghost_timeout_qc[qc_id].ghost_view + 1
        }
    }

    def new_view_verify(new_view: NewView): bool = all {
        // Check that the justification is valid.
        new_view.justification.justification_verify()
    }

    // This function verifies an aggregated signature and checks it has enough weight.
    // In here we need to not only check the signature, but also that
    // it has enough weight beyond it.
    pure def aggregate_verify(agg_sig: AggregateSignature): bool = and {
        agg_sig.size() >= QUORUM_WEIGHT,
        agg_sig.subseteq(REPLICAS),
    }

    def signed_timeout_vote_verify(signed_vote: SignedTimeoutVote): bool = all {
        signed_vote.vote.view >= 0,
        match (signed_vote.high_commit_qc) {
        | None =>
                signed_vote.vote.high_commit_qc_view.is_none()
        | Some(qc_id) =>
            val qc = ghost_commit_qc[qc_id]
            and {
                Some(qc.vote.view) == signed_vote.vote.high_commit_qc_view,
                commit_qc_verify(qc),
            }
        },
        timeout_vote_verify(signed_vote.vote)
    }

    pure def signed_commit_vote_verify(signed_vote: SignedCommitVote): bool = all {
        commit_vote_verify(signed_vote.vote),
        // we can do this check as a correct replica knows the public keys of the other replicas
        REPLICAS.contains(signed_vote.sig),
    }

    // Commit vote verification that a replica can do locally
    pure def commit_vote_verify(vote: CommitVote): bool = all {
        vote.view >= 0,
        vote.block_number >= 0,
    }

    // Timeout vote verification that a replica can do locally
    pure def timeout_vote_verify(vote: TimeoutVote): bool = all {
        vote.view >= 0,
        match vote.high_vote {
            | None => true
            | Some(high_vote) => and {
                    commit_vote_verify(high_vote),
                    // There are two invariants that are easy to check but aren't required for correctness:
                    vote.view >= high_vote.view,
                    match vote.high_commit_qc_view {
                        | None => true
                        | Some(v) => high_vote.view >= v
                    }
                }
        }
    }

    // Commit QC verification that a replica can do locally
    pure def commit_qc_verify(qc: CommitQC): bool = and {
        commit_vote_verify(qc.vote),
        // In here we need to not only check the signature, but also that
        // it has enough weight beyond it.
        qc.agg_sig.aggregate_verify()
    }

    // Timeout QC verification that a replica can do locally
    def timeout_qc_verify(qc: TimeoutQC): bool =  {
        val votes = qc.votes.keys().map(pk => qc.votes.get(pk))
        and {
            // all votes are valid
            votes.forall(v => timeout_vote_verify(v)),
            // Check that all votes have the same view.
            // We are using the ghost view to do this check.
            votes.forall(v => v.view == qc.ghost_view),
            // In here we need to not only check the signature, but also that
            // it has enough weight beyond it.
            qc.agg_sig.aggregate_verify(),
            // It is essential that no vote is missing
            qc.votes.keys() == qc.agg_sig,
            // Check that all votes have the same view and get the highest commit QC view of the timeout QC.
            val high_qc_view: Option[ViewNumber] =
                votes.fold(VIEW_NUMBER_NONE, (prev_max_opt, v) => match (prev_max_opt) {
                | None =>
                    v.high_commit_qc_view
                | Some(prev_max) =>
                    if (v.high_commit_qc_view.option_has(qcv => qcv > prev_max)) {
                        v.high_commit_qc_view
                    } else {
                        prev_max_opt
                    }
                })
            // Get the high commit QC of the timeout QC. We compare the high QC field of all
            // timeout votes in the QC, and get the highest one, if it exists.
            // We then need to verify that it is valid. We don't need to verify the commit QCs
            // of the other timeout votes, since they don't have any influence in the result.
            match (qc.high_commit_qc) {
                | None =>
                    high_qc_view.is_none()

                | Some(high_qc_id) =>
                    val high_qc = ghost_commit_qc[high_qc_id]
                    and {
                        high_qc_view == Some(high_qc.vote.view),
                        commit_qc_verify(high_qc),
                    }
            }
        }
    }

    // This returns the block number that is implied by this justification.
    // If the justification requires a block reproposal, it also returns
    // the hash of the block that must be reproposed.
    def get_implied_block(justification: Justification): (BlockNumber, Option[BlockHash]) = {
        match (justification) {
            | Commit(qc_id) => {
                // The previous proposal was finalized, so we can propose a new block.
                val qc = ghost_commit_qc[qc_id]
                (qc.vote.block_number + 1, None)
            }

            | Timeout(qc_id) => {
                val qc = ghost_timeout_qc[qc_id]
                // Get the high vote of the timeout QC, if it exists. We check if there are
                // timeout votes with at least an added weight of SUBQUORUM_WEIGHT,
                // that have a high vote field for the same block. A QC can have
                // 0, 1 or 2 such blocks.
                // If there's only 1 such block, then we say the QC has a high vote.
                // If there are 0 or 2 such blocks, we say the QC has no high vote.
                val qc_high_vote: Option[CommitVote] = qc.high_vote()
                // Get the high commit QC of the timeout QC. We compare the high QC field of
                // all timeout votes in the QC, and get the highest one, if it exists.
                match (qc_high_vote) {
                | Some(hv) =>
                        match (qc.high_commit_qc) {
                        | Some(hqc_id) =>
                            val hqc = ghost_commit_qc[hqc_id]
                            if (hv.block_number > hqc.vote.block_number) {
                                // There was some proposal last view that might have been finalized.
                                // We need to repropose it.
                                (hv.block_number, Some(hv.block_hash))
                            } else {
                                // Either the previous proposal was finalized or we know for certain
                                // that it couldn't have been finalized. Either way, we can propose
                                // a new block.
                                (hqc.vote.block_number + 1, None)
                            }

                        | None =>
                            // There was some proposal last view that might have been finalized.
                            // We need to repropose it.
                            (hv.block_number, Some(hv.block_hash))
                        }

                | None => 
                        // Either the previous proposal was finalized or we know for certain
                        // that it couldn't have been finalized. Either way, we can propose
                        // a new block.
                        match (qc.high_commit_qc) {
                            | Some(hqc_id) =>
                                val hqc = ghost_commit_qc[hqc_id]
                                (hqc.vote.block_number + 1, None)

                            | None => (0, None)
                        }
                }
            }
        }
    }

    // Get the high vote of the QC, if it exists. We check if there are
    // timeout votes with at least an added weight of SUBQUORUM_WEIGHT,
    // that have a high vote field for the same block. A QC can have
    // 0, 1 or 2 such blocks.
    // If there's only 1 such block, then we say the QC has a high vote.
    // If there are 0 or 2 such blocks, we say the QC has no high vote.    
    pure def high_vote(qc: TimeoutQC): Option[CommitVote] = {
        // find all kinds of high votes among the QC votes
        pure val high_votes = qc.votes.keys().map(pk => qc.votes.get(pk).high_vote)
        // compute the pair (high_vote, nr_of_votes) for each high_vote in high_votes
        pure val high_votes_sizes =
            high_votes.map(hv => (hv, qc.votes.keys().filter(pk => qc.votes.get(pk).high_vote == hv).size()))
        // Select all high_vote values that go over the threshold.
        val subquorums = high_votes_sizes.fold([], (hv_list, hvs) => {
            if (hvs._2 >= SUBQUORUM_WEIGHT) {
                hv_list.append(hvs._1)
            } else {
                hv_list
            }
        })
        // if there only one high_vote possible, return it
        if (subquorums.length() == 1) {
            subquorums[0]
        } else {
            None
        }
    }

    // -------------------------------------------------------------------------
    // Invariants to check

    // No two correct replicas disagree on the committed blocks.
    val agreement_inv = tuples(CORRECT, CORRECT).forall(((id1, id2)) => {
        val blocks1 = replica_state.get(id1).committed_blocks
        val blocks2 = replica_state.get(id2).committed_blocks
        or {
            // ignore this case, as there is a symmetric one
            blocks1.length() > blocks2.length(),
            // the shorter sequence is a prefix of the longer one
            blocks1.indices().forall(i => blocks1[i].block == blocks2[i].block)
        }
    })

    // every entry in committed_blocks must have a justification
    val committed_blocks_have_justification_inv = {
        CORRECT.forall(id => {
            val committed_blocks = replica_state.get(id).committed_blocks
            committed_blocks.indices().forall(block_number => {
                val block_hash = committed_blocks[block_number].block
                ghost_justifications.keys().exists(((id2, view)) => and {
                    id2 == id,
                    match (ghost_justifications.get((id2, view))) {
                        | Timeout(qc_id) =>
                            val qc = ghost_timeout_qc[qc_id]
                            qc.high_commit_qc.option_has(hqc_id =>
                                val hqc = ghost_commit_qc[hqc_id]
                                and {
                                    hqc.vote.block_number == block_number,
                                    hqc.vote.block_hash == block_hash,
                                    view == hqc.vote.view or view == hqc.vote.view + 1,
                                })

                        | Commit(qc_id) =>
                            val qc = ghost_commit_qc[qc_id]
                            and {
                                qc.vote.block_number == block_number,
                                qc.vote.block_hash == block_hash,
                                view == qc.vote.view or view == qc.vote.view + 1,
                            }
                    }
                })
            })
        })
    }

    // a correct proposer should not equivocate for the same view
    val no_proposal_equivocation_inv = tuples(msgs_proposal, msgs_proposal).forall(((m1, m2)) => or {
        not(m1.justification.view() == m2.justification.view() and m1.sig == m2.sig),
        FAULTY.contains(m1.sig),
        m1.block == m2.block,
    })

    // a correct proposer should not send two commits in the same view
    val no_commit_equivocation_inv = tuples(msgs_signed_commit, msgs_signed_commit).forall(((m1, m2)) => or {
        not(m1.vote.view == m2.vote.view and m1.sig == m2.sig),
        FAULTY.contains(m1.sig),
        m1.vote.block_hash == m2.vote.block_hash and m1.vote.block_number == m2.vote.block_number,
    })

    // a correct proposer should not send two timeouts in the same view
    val no_timeout_equivocation_inv = tuples(msgs_signed_timeout, msgs_signed_timeout).forall(((m1, m2)) => or {
        not(m1.vote.view == m2.vote.view and m1.sig == m2.sig),
        FAULTY.contains(m1.sig),
        and {
            m1.vote.high_vote == m2.vote.high_vote,
            m1.high_commit_qc == m2.high_commit_qc,
            m1.vote.high_commit_qc_view == m2.vote.high_commit_qc_view,
        }
    })

    // a correct proposer should not send two new views in the same view
    val no_new_view_equivocation_inv = {
            tuples(msgs_new_view, msgs_new_view).forall(((m1, m2)) => or {
            FAULTY.contains(m1.sig) or FAULTY.contains(m2.sig),
            m1.sig != m2.sig,
            m1.justification.view() != m2.justification.view(),
            m1.justification == m2.justification,
        })
    }

    // View change only on QC.
    // Replicas should only change view when they receive a QC (commit or timeout) that justifies it.
    // This invariant checks that every justification is verifiable, and that it does not come from the future.
    val view_justification_inv = {
        ghost_justifications.keys().forall(((id, v)) => and {
            1 <= v and v <= replica_state.get(id).view,
            justification_verify(ghost_justifications.get((id, v))),
            leader.keys().contains(v),
        })
    }

    // If a correct replica reaches a view, then, collectively, the correct replicas
    // have justifications for the views below.
    val view_justification_continuous_inv = {
        VIEWS.forall(v => or {
            v == 0,
            CORRECT.forall(id => replica_state.get(id).view < v),
            ghost_justifications.keys().exists(((id, other_view)) => other_view == v)
        })
    }

    // Checks that for every justification stored by a replica `r1`,
    // the votes by the correct replicas are backed by sent messages.
    val justification_is_supported_inv: bool = {
        ghost_justifications.keys().forall(((pk1, replica_view)) => {
            match (ghost_justifications.get((pk1, replica_view))) {
                | Timeout(qc_id) => and {
                    replica_view > 0,
                    // Only check that quorums are supported by sent votes.
                    // The quorum invariants are checked in view_justification_inv.
                    val qc = ghost_timeout_qc[qc_id]
                    qc.votes.keys().forall(pk2 => or {
                        FAULTY.contains(pk2),
                        val vote = qc.votes.get(pk2)
                        msgs_signed_timeout.exists(signed_vote => signed_vote.vote == vote)
                    })
                }

                | Commit(qc_id) => and {
                    replica_view > 0,
                    // Only check that quorums are supported by the sent votes.
                    // The quorum invariants are checked in view_justification_inv.
                    val qc = ghost_commit_qc[qc_id]
                    qc.agg_sig.forall(pk2 => or {
                        FAULTY.contains(pk2),
                        msgs_signed_commit.exists(signed_vote => signed_vote.vote == qc.vote)
                    })                    
                }
            }
        })
    }

    // there are no two high votes supported by subquorum weight (2 * F + 1) in a single TimeoutQC
    def one_high_vote_in_timeout_qc_inv: bool = {
        def one_high_vote(qc_id: int): bool = {
            val qc = ghost_timeout_qc[qc_id]
            val votes = qc.votes.keys().map(pk => qc.votes.get(pk))
            val high_votes = votes.map(v => v.high_vote)
            // compute the pair (high_vote, nr_of_votes) for each high_vote in high_votes
            val high_vote_count =
                high_votes.mapBy(hv => votes.filter(v => v.high_vote == hv).size())
            tuples(high_votes, high_votes).forall(((hv1, hv2)) => or {
                hv1 == hv2,
                high_vote_count.get(hv1) < SUBQUORUM_WEIGHT,
                high_vote_count.get(hv2) < SUBQUORUM_WEIGHT,
            })
        }

        replica_state.keys().forall(id => {
            val timeout_qc = replica_state.get(id).high_timeout_qc
            timeout_qc.is_none() or timeout_qc.option_has(one_high_vote)
        })
    }

    // There are no two quorums for different block hashes in one view.
    // Importantly, faulty replicas may send multiple votes with different block numbers.
    def one_commit_quorum_inv: bool = {
        def one_commit_quorum_per_view(fixed_view: ViewNumber): bool = {
            val votes = msgs_signed_commit.filter(v => v.vote.view == fixed_view)
            val hashes = votes.map(v => v.vote.block_hash)
            val vote_count =
                hashes.mapBy(h => votes.filter(v => v.vote.block_hash == h).map(v => v.sig).size())
            tuples(hashes, hashes).forall(((h1, h2)) => or {
                h1 == h2,
                vote_count.get(h1) < QUORUM_WEIGHT,
                vote_count.get(h2) < QUORUM_WEIGHT,
            })
        }

        msgs_signed_commit
            .map(v => v.vote.view)
            .forall(v => one_commit_quorum_per_view(v))
    }

    // integrity of high_commit_qc for all replicas
    val all_replicas_high_commit_qc_inv: bool = {
        CORRECT.forall(id => {
            aux_commit_qc_inv(replica_state.get(id).high_commit_qc)
        })
    }

    // basic integrity invariant for commit_qc
    def aux_commit_qc_inv(qc_opt: Option[QcId]): bool = {
        match (qc_opt) {
            | None => true
            | Some(qc_id) =>
                val qc = ghost_commit_qc[qc_id]
                and {
                    qc.vote.view >= 0,
                    qc.vote.block_number >= 0,
                    VALID_BLOCKS.contains(qc.vote.block_hash),
                    qc.agg_sig.aggregate_verify()
                }
        }
    }

    // integrity of high_timeout_qc for all replicas
    val all_replicas_high_timeout_qc_inv: bool = {
        CORRECT.forall(id => {
            aux_timeout_qc_inv(replica_state.get(id).high_timeout_qc)
        })
    }

    // integrity invariant for TimeoutQC
    def aux_timeout_qc_inv(qc_opt: Option[QcId]): bool = {
        match (qc_opt) {
            | None => true
            | Some(qc_id) => 
                val qc = ghost_timeout_qc[qc_id]
                val votes = qc.votes.keys().map(pk => qc.votes.get(pk))
                val public_keys = qc.votes.keys()
                and {
                    qc.ghost_view >= 0,
                    qc.agg_sig.aggregate_verify(),
                    public_keys == qc.agg_sig,
                    votes.forall(v => v.timeout_vote_verify()),
                    // make sure that all votes are cast for the same view
                    votes.forall(v => qc.ghost_view == v.view),
                    // The commit quorum certificate with the highest view that all replicas in this
                    // QC have observed, if any. It MUST match the highest `high_commit_qc_view` in `votes`.
                    match (qc.high_commit_qc) {
                    | None =>
                        votes.forall(v => v.high_commit_qc_view.is_none())

                    | Some(commit_qc_id) =>
                        val commit_qc = ghost_commit_qc[commit_qc_id]
                        and {
                            votes.forall(v => v.view <= commit_qc.vote.view),
                            votes.exists(v => v.view == commit_qc.vote.view)
                        }
                    }
                }
        }
    }

    // a state invariant to guarantee that correct replicas do not send malformed timeout votes
    def msgs_signed_timeout_inv: bool = {
        msgs_signed_timeout.forall(v => {
            CORRECT.contains(v.sig) implies signed_timeout_vote_verify(v)
        })
    }

    // a state invariant to guarantee that correct replicas do not send malformed signed commit votes
    def msgs_signed_commit_inv: bool = {
        msgs_signed_commit.forall(v => or {
            FAULTY.contains(v.sig),
            aux_commit_vote_inv(v),
        })
    }

    // basic integrity invariant for commit vote
    def aux_commit_vote_inv(signed_vote: SignedCommitVote): bool = and {
        signed_vote.vote.view >= 0,
        signed_vote.vote.block_number >= 0,
        REPLICAS.contains(signed_vote.sig),
        VALID_BLOCKS.contains(signed_vote.vote.block_hash),
    }

    // integrity of high_vote for all replicas
    val all_replicas_high_vote_inv: bool = {
        CORRECT.forall(id => {
            val replica_high_vote = replica_state.get(id).high_vote
            val replica_view = replica_state.get(id).view
            match replica_high_vote {
                | None =>
                    // no commit votes sent so far
                    msgs_signed_commit.forall(sv => sv.sig != id)

                | Some(high_vote) => all {
                    // this vote is valid
                    aux_commit_vote_inv({vote: high_vote, sig: id }),
                    // this vote was signed and sent
                    { vote: high_vote, sig: id }.in(msgs_signed_commit),
                    // there is no higher signed commit
                    msgs_signed_commit.forall(sv => or {
                        sv.sig != id,
                        sv.vote.view <= high_vote.view
                    }),
                    // views of the replica can't be lower than its `high_vote`'s view
                    replica_view >= high_vote.view,
                    match (replica_state.get(id).high_commit_qc) {
                        | None => true
                        | Some(qc_id) =>
                            val qc = ghost_commit_qc[qc_id]
                            high_vote.view >= qc.vote.view
                    }
                }
            }
        })
    }

    // a high vote is the highest among the votes sent by a replica
    val timeout_high_vote_is_highest_inv: bool = {
        msgs_signed_timeout.forall(tv => or {
            FAULTY.contains(tv.sig),
            // get the commit messages below the view of the timeout vote
            val commit_votes: Set[SignedCommitVote] =
                msgs_signed_commit.filter(sv => and {
                    sv.sig == tv.sig,
                    sv.vote.view <= tv.vote.view,
                })
            // find the highest among them, if any
            val highest: Option[CommitVote] =
                commit_votes
                    .fold(COMMIT_VOTE_NONE, (hv_opt, sv) => {
                    match (hv_opt) {
                        | None => Some(sv.vote)
                        | Some(hv) => if (hv.view > sv.vote.view) Some(hv) else Some(sv.vote)
                    }
                })
            // it should be equal to high_vote
            tv.vote.high_vote == highest
        })
    }

    // make sure that a correct replica is voting for a block that can be retrieved
    // from sufficiently many other correct replicas
    val all_replicas_commit_is_backed_by_subquorum_inv: bool = {
        msgs_signed_commit.forall(signed_vote => or {
            // if signed_vote is sent by a faulty replica, ignore it
            FAULTY.contains(signed_vote.sig),
            // if signed_vote is sent by a correct replica,
            // count the number of correct replicas that have the previous block
            val pair = (signed_vote.vote.block_number, signed_vote.vote.block_hash)
            val correct_with_block = CORRECT.filter(id => {
                replica_state.get(id).cached_proposals.contains(pair)
            })
            or {
                // TODO: use SUBQUORUM_WEIGHT
                correct_with_block.size() >= QUORUM_WEIGHT,
                // the first block does not need a quorum
                signed_vote.vote.block_number <= 1,
            }
        })
    }

    // All invariants
    // TODO: add the latest invariant about quorum of votes for each proposed block
    val all_invariants = all {
        agreement_inv,
        committed_blocks_have_justification_inv,
        no_proposal_equivocation_inv,
        no_commit_equivocation_inv,
        no_timeout_equivocation_inv,
        no_new_view_equivocation_inv,
        view_justification_inv,
        justification_is_supported_inv,
        one_high_vote_in_timeout_qc_inv,
        one_commit_quorum_inv,
        all_replicas_high_commit_qc_inv,
        all_replicas_high_timeout_qc_inv,
        msgs_signed_timeout_inv,
        msgs_signed_commit_inv,
        all_replicas_high_vote_inv,
        timeout_high_vote_is_highest_inv,
    }

    // -------------------------------------------------------------------------
    // Invariants related to liveness

    // a correct replica should be able to commit at least one block once in every 5F + 1 views
    val block_progress_inv = {
        CORRECT.forall(id => {
            val state = replica_state.get(id)
            state.view < (5 * F + 1) * (state.committed_blocks.length() + 1)
        })
    }

    // -------------------------------------------------------------------------
    // Falsy invariants to check reachability of certain states

    // check this invariant to see an example of reaching PhaseCommit
    val phase_commit_example = CORRECT.forall(id => replica_state.get(id).phase != PhaseCommit)

    // check this invariant to see an example of a non-empty timeout QC
    val high_timeout_qc_example = CORRECT.forall(id => and {
        match (replica_state.get(id).high_timeout_qc) {
            | None => true
            | Some(qc_id) =>
                val qc = ghost_timeout_qc[qc_id]
                qc.votes.keys().size() == 0
        }
    })

    // check this invariant to see an example of having a timeout quorum:
    val timeout_qc_example = msgs_signed_timeout.map(m => (m.sig, m.vote.view)).size() < QUORUM_WEIGHT

    // check this invariant to see an example of having at least QUORUM_WEIGHT message sent for the same view
    val commit_qc_example = VIEWS.forall(v => {
        msgs_signed_commit.filter(signed_vote => signed_vote.vote.view == v).size() < QUORUM_WEIGHT
    })

    // Check this invariant to see an example of a justification.
    // (Except view 0, which is just a bootstrapping view.)
    val justification_example =
        ghost_justifications.keys().exists(((id, v)) => v > 0)

    // check this invariant to see an example of having a finalized block:
    val one_block_example = CORRECT.forall(id => {
        not(replica_state.get(id).committed_blocks.length() > 0)
    })

    // Check this invariant to see an example of two blocked being finalized by a single replica.
    // This example is important, as finalizing just 1 block does not involve syncing.
    val two_chained_blocks_example = CORRECT.forall(id => {
        not(replica_state.get(id).committed_blocks.length() > 1)
    })

    // check this invariant to see an example of a commit-QC equal to None
    val high_commit_qc_example = CORRECT.forall(id => {
        val self = replica_state.get(id)
        is_none(self.high_commit_qc)
    })

    // check this invariant to see an example of a replica reaching a view > 2
    val views_over_2_example = CORRECT.forall(id => {
        replica_state.get(id).view <= 2
    })
}