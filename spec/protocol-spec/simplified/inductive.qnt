// An inductive invariant for the agreement property.
//
// To make sure that ind_inv is inductive w.r.t. agreement_inv, we need to run three queries:
//
// JVM_ARGS=-Xmx20G quint verify --server-endpoint=localhost:8080 \
//   --apalache-config=apalache-inductive.json \
//   --max-steps=0 --init=init --invariant=ind_inv n6f1b1_inductive.qnt
//
// JVM_ARGS=-Xmx20G quint verify --server-endpoint=localhost:8081 \
//   --apalache-config=apalache-inductive.json \
//   --max-steps=1 --init=ind_init --step=post_ind_inv --invariant=agreement_inv n6f1b1_inductive.qnt
//
// JVM_ARGS=-Xmx20G quint verify --server-endpoint=localhost:8082 \
//   --apalache-config=apalache-inductive.json --max-steps=1 --init=ind_init \
//   --step=step --invariant=ind_inv n6f1b1_inductive.qnt
//
// Since checking the inductive invariant requires significant computational resources,
// we check the pieces of the inductive invariant in parallel on a beefy machine:
//
// ../experiments/quint-inductive.sh n6f1b1_inductive.qnt 13 23 40 300 ind_inv ind_init step
//
// Igor Konnov 2024-2025 (for Matter Labs)
module inductive {
    import types.* from "./types"
    import option.* from "../option"
    
    // parameters
    const I_N: int
    const I_F: int
    const I_CORRECT: Set[ReplicaId]
    const I_FAULTY: Set[ReplicaId]
    const I_VIEWS: Set[ViewNumber]
    const I_BLOCK_NUMBERS: Set[BlockNumber]
    const I_VALID_BLOCKS: Set[Block]
    const I_INVALID_BLOCKS: Set[Block]

    // we restrict the number of CommitQC and TimeoutQC. Otherwise, the search space is too large.
    const MAX_COMMIT_QC: int
    const MAX_TIMEOUT_QC: int

    pure val IALL_BLOCKS = I_VALID_BLOCKS.union(I_INVALID_BLOCKS)
    pure val IREPLICAS = I_CORRECT.union(I_FAULTY)

    import replica(
        N = I_N,
        F = I_F,
        CORRECT = I_CORRECT,
        FAULTY = I_FAULTY,
        VIEWS = I_VIEWS,
        VALID_BLOCKS = I_VALID_BLOCKS,
        INVALID_BLOCKS = I_INVALID_BLOCKS
    ) as r from "./replica"

    var step_no: int
    // if prophecy_magic_view > 0,
    // a number of good conditions have been met in prophecy_magic_view (see below)
    var prophecy_magic_view: int

    // definitions

    pure def commit_vote_shape_inv(v: CommitVote): bool = and {
        I_VIEWS.contains(v.view),
        I_BLOCK_NUMBERS.contains(v.block_number),
        IALL_BLOCKS.contains(v.block_hash),
    }

    pure def commit_qc_shape_inv(qc: CommitQC): bool = and {
        commit_vote_shape_inv(qc.vote),
        qc.agg_sig.subseteq(IREPLICAS),
    }

    pure def timeout_vote_shape_inv(v: TimeoutVote): bool = and {
        I_VIEWS.contains(v.view),
        match (v.high_vote) {
        | Some(hv) => commit_vote_shape_inv(hv)
        | None => true
        },
        match (v.high_commit_qc_view) {
        | Some(i) => I_VIEWS.contains(i)
        | None => true
        },
    }    

    pure def timeout_qc_shape_inv(qc: TimeoutQC, commit_qc_size: int): bool = and {
        qc.votes.keys().forall(id => and {
            IREPLICAS.contains(id),
            timeout_vote_shape_inv(qc.votes.get(id)),
        }),
        qc.agg_sig.subseteq(IREPLICAS),
        match (qc.high_commit_qc) {
        | Some(i) => (0 <= i) and (i < commit_qc_size)
        | None => true
        },
        I_VIEWS.contains(qc.ghost_view),
    }

    pure def signed_commit_vote_shape_inv(sv: SignedCommitVote): bool = and {
        IREPLICAS.contains(sv.sig),
        commit_vote_shape_inv(sv.vote)
    }

    pure def signed_timeout_vote_shape_inv(sv: SignedTimeoutVote): bool = and {
        IREPLICAS.contains(sv.sig),
        timeout_vote_shape_inv(sv.vote),
        match (sv.high_commit_qc) {
        | Some(qc_id) => 0 <= qc_id and qc_id <= MAX_COMMIT_QC
        | None => true
        },
    }

    pure val COMMIT_VOTES: Set[CommitVote] =
        tuples(I_VIEWS.exclude(Set(0)), I_BLOCK_NUMBERS, IALL_BLOCKS)
            .map(((v, bn, bh)) => {
                { view: v, block_number: bn, block_hash: bh }
            })

    pure val SIGNED_COMMIT_VOTES: Set[SignedCommitVote] =
        tuples(I_VIEWS.exclude(Set(0)), I_BLOCK_NUMBERS, IALL_BLOCKS, IREPLICAS)
            .map(((v, bn, bh, sig)) => {
                { vote: { view: v, block_number: bn, block_hash: bh }, sig: sig }
            })

    pure val COMMIT_QCS: Set[CommitQC] = tuples(COMMIT_VOTES, powerset(IREPLICAS)).map(((cv, sigs)) => {
        { vote: cv, agg_sig: sigs }
    })

    pure val COMMIT_VOTES_OR_NONE: Set[Option[CommitVote]] = COMMIT_VOTES.map(v => Some(v)).union(Set(COMMIT_VOTE_NONE))

    pure val VIEW_NUMBERS_OR_NONE: Set[Option[ViewNumber]] = I_VIEWS.map(v => Some(v)).union(Set(VIEW_NUMBER_NONE))

    pure val TIMEOUT_VOTES: Set[TimeoutVote] =
        tuples(I_VIEWS, COMMIT_VOTES_OR_NONE, VIEW_NUMBERS_OR_NONE).map(((v, hv, hcv)) =>
            { view: v, high_vote: hv, high_commit_qc_view: hcv }
        )

    pure val SIGNED_TIMEOUT_VOTES: Set[SignedTimeoutVote] =
        tuples(I_VIEWS, COMMIT_VOTES_OR_NONE, VIEW_NUMBERS_OR_NONE, IREPLICAS, 0.to(MAX_COMMIT_QC), Bool)
            .map(((v, hv, hcv, id, qc_id, qc_is_some)) =>
                {
                    vote: { view: v, high_vote: hv, high_commit_qc_view: hcv },
                    sig: id,
                    high_commit_qc: if (qc_is_some) Some(qc_id) else None
                }
        )

    pure val COMMIT_VOTE_SAMPLE: CommitVote = {
        view: 0, block_number: 0, block_hash: "val_b0",
    }        

    pure val TIMEOUT_VOTE_SAMPLE: TimeoutVote = {
        view: 0,
        high_vote: Some(COMMIT_VOTE_SAMPLE),
        high_commit_qc_view: Some(0),
    }        

    pure val SIGNED_TIMEOUT_VOTE_SAMPLE: SignedTimeoutVote = {
        sig: "n0",
        vote: TIMEOUT_VOTE_SAMPLE,
        high_commit_qc: Some(0),
    }

    pure val NEW_VIEWS: Set[NewView] =
        tuples(IREPLICAS, 0.to(MAX_COMMIT_QC).union(0.to(MAX_TIMEOUT_QC)), Bool)
            .map(((id, qc_id, is_commit_id)) =>
                { sig: id, justification: if (is_commit_id) Commit(qc_id) else Timeout(qc_id) }
        )

    pure val NEW_VIEW_SAMPLE: NewView = {
        sig: "n0",
        justification: Commit(0),
    }

    pure val PROPOSALS: Set[Proposal] =
        tuples(IALL_BLOCKS, Bool, 0.to(MAX_COMMIT_QC).union(0.to(MAX_TIMEOUT_QC)), Bool, IREPLICAS, I_BLOCK_NUMBERS)
            .map(((bh, is_block_some, qc_id, is_commit_id, id, bn)) => {
                block: if (is_block_some) Some(bh) else None,
                justification: if (is_commit_id) Commit(qc_id) else Timeout(qc_id),
                sig: id,
                ghost_block_number: bn
            })
    
    pure val PROPOSAL_SAMPLE: Proposal = {
        block: Some("val_b0"),
        justification: Commit(0),
        sig: "n0",
        ghost_block_number: 0,
    }

    pure def proposal_shape_inv(sv: Proposal): bool = and {
        IREPLICAS.contains(sv.sig),
        I_BLOCK_NUMBERS.contains(sv.ghost_block_number),
        match (sv.block) {
        | Some(b) => IALL_BLOCKS.contains(b)
        | None => true
        },
        match (sv.justification) {
        | Commit(qc_id) => 0 <= qc_id and qc_id <= MAX_COMMIT_QC
        | Timeout(qc_id) => 0 <= qc_id and qc_id <= MAX_TIMEOUT_QC
        },
    }

    pure val JUSTIFICATIONS: Set[Justification] =
        tuples(0.to(MAX_COMMIT_QC).union(0.to(MAX_TIMEOUT_QC)), Bool)
            .map(((qc_id, is_commit_id)) => if (is_commit_id) Commit(qc_id) else Timeout(qc_id))
    
    pure val COMMIT_QC_ID_OR_NONE: Set[Option[QcId]] =
        0.to(MAX_COMMIT_QC).map(i => Some(i)).union(Set(HIGH_COMMIT_QC_NONE))

    pure val TIMEOUT_QC_ID_OR_NONE: Set[Option[QcId]] =
        0.to(MAX_TIMEOUT_QC).map(i => Some(i)).union(Set(HIGH_TIMEOUT_QC_NONE))

    pure val CACHED_PROPOSALS: Set[Set[(BlockNumber, BlockHash)]] =
        tuples(I_BLOCK_NUMBERS, IALL_BLOCKS).powerset()

    pure val COMMITTED_BLOCKS: Set[CommittedBlock] =
        tuples(IALL_BLOCKS, 0.to(MAX_COMMIT_QC))
            .map(((b, qc_id)) => { block: b, commit_qc: qc_id })

    // the set of all singleton committed blocks + the empty list
    pure val COMMITTED_BLOCKS_0_OR_1: Set[List[CommittedBlock]] =
        COMMITTED_BLOCKS.map(b => [b]).union(Set([]))

    pure val PHASES = Set(PhasePrepare, PhaseCommit, PhaseTimeout)

    // the initialization from the replica module
    action init = all{
        step_no' = 0,
        nondet mv = I_VIEWS.oneOf()
        prophecy_magic_view' = mv,
        r::init,
    }

    // non-deterministically initialize the states
    action ind_init = {
        // non-deterministically initialize ghost_commit_qc
        nondet commit_qcs = 0.to(MAX_COMMIT_QC).setOfMaps(COMMIT_QCS).oneOf()
        val commit_qc = commit_qcs.keys().fold([], (lst, i) => lst.append(commit_qcs.get(i)))
        // non-deterministically initialize ghost_timeout_qc
        nondet timeout_qc_agg_sig = 0.to(MAX_TIMEOUT_QC).setOfMaps(powerset(IREPLICAS)).oneOf()
        nondet timeout_qc_votes = tuples(0.to(MAX_TIMEOUT_QC), IREPLICAS).setOfMaps(TIMEOUT_VOTES).oneOf()
        nondet timeout_qc_hcq =
            0.to(MAX_TIMEOUT_QC)
              .setOfMaps(0.to(MAX_COMMIT_QC).map(id => Some(id)).union(Set(HIGH_COMMIT_QC_NONE)))
              .oneOf()
        nondet timeout_qc_view = 0.to(MAX_TIMEOUT_QC).setOfMaps(I_VIEWS).oneOf()
        val timeout_qc =
            0.to(MAX_TIMEOUT_QC)
                .fold([], (lst, i) => lst.append({
                    votes: timeout_qc_agg_sig.get(i).mapBy(id => timeout_qc_votes.get((i, id))),
                    agg_sig: timeout_qc_agg_sig.get(i),
                    high_commit_qc: timeout_qc_hcq.get(i),
                    ghost_view: timeout_qc_view.get(i),
                }))
        // non-deterministically initialize msgs_signed_commit
        nondet signed_commits = SIGNED_COMMIT_VOTES.powerset().oneOf()
        // Non-deterministically initialize replica_state:
        //
        // Pick maps for the individual fields, to avoid combinatorial explosion of tuples.
        nondet state_views = I_CORRECT.setOfMaps(I_VIEWS).oneOf()
        nondet state_phases = I_CORRECT.setOfMaps(PHASES).oneOf()
        nondet high_votes = I_CORRECT.setOfMaps(COMMIT_VOTES_OR_NONE).oneOf()
        nondet high_commit_qcs = I_CORRECT.setOfMaps(COMMIT_QC_ID_OR_NONE).oneOf()
        nondet high_timeout_qcs = I_CORRECT.setOfMaps(TIMEOUT_QC_ID_OR_NONE).oneOf()
        nondet cached_proposals = I_CORRECT.setOfMaps(CACHED_PROPOSALS).oneOf()
        nondet committed_blocks = I_CORRECT.setOfMaps(COMMITTED_BLOCKS_0_OR_1).oneOf()
        val replica_s: ReplicaId -> ReplicaState =
            I_CORRECT.mapBy(id =>
                {
                    view: state_views.get(id),
                    phase: state_phases.get(id),
                    high_vote: high_votes.get(id),
                    high_commit_qc: high_commit_qcs.get(id),
                    high_timeout_qc: high_timeout_qcs.get(id),
                    cached_proposals: cached_proposals.get(id),
                    committed_blocks: committed_blocks.get(id),
                })
        // non-deterministically choose the leader function
        nondet ldr = I_VIEWS.setOfMaps(IREPLICAS).oneOf()
        // non-deterministically choose replica_view and proposer_view
        nondet rv = I_CORRECT.setOfMaps(I_VIEWS).oneOf()
        nondet pv = I_CORRECT.setOfMaps(I_VIEWS).oneOf()
        // guess prophecy_correct_proposals
        nondet i_prophecy_correct_proposals = I_VIEWS.setOfMaps(I_VALID_BLOCKS).oneOf()
        all {
            r::leader' = ldr,
            r::replica_view' = rv,
            r::proposer_view' = pv,
            r::replica_state' = replica_s,
            r::ghost_commit_qc' = commit_qc,
            r::ghost_timeout_qc' = timeout_qc,
            r::msgs_signed_commit' = signed_commits,
            {
                // Pass the upper bound of 10 directly (this is a limitation of Quint)
                // To understand why this makes sense, see discussion in the paper.
                nondet msgs_signed_timeout = generate(10, Set(SIGNED_TIMEOUT_VOTE_SAMPLE))
                all {
                    msgs_signed_timeout.forall(signed_timeout_vote_shape_inv),
                    r::msgs_signed_timeout' = msgs_signed_timeout,
                }
            },
            {
                nondet msgs_new_view = generate(5, Set(NEW_VIEW_SAMPLE))
                r::msgs_new_view' = msgs_new_view
            },
            {
                // Pass the upper bound of 10 directly (this is a limitation of Quint)
                nondet msgs_proposal = generate(5, Set(PROPOSAL_SAMPLE))
                all {
                    msgs_proposal.forall(proposal_shape_inv),
                    r::msgs_proposal' = msgs_proposal,
                }
            },
            r::ghost_justifications' = Map(),
            r::ghost_step' = AnyStep,
            r::prophecy_correct_proposals' = i_prophecy_correct_proposals,
            nondet mv = I_VIEWS.oneOf()
            prophecy_magic_view' = mv,
            step_no' = 0,
        }
    }

    // since Quint does not support postcondition directly,
    // we have to introduce a stuttering step that assumes the postcondition
    action post_ind_inv = all {
        ind_inv,
        unchanged_all,
    }

    // we only look at the step that has ind_inv as the pre-condition
    action step = all {
        ind_inv,
        r::step,
        prophecy_magic_view' = prophecy_magic_view,
        step_no' = step_no + 1,
    }

    action correct_step = all{
        ind_inv,
        r::correct_step,
        prophecy_magic_view' = prophecy_magic_view,
        step_no' = step_no + 1,
    }

    // this is the main safety property that we are interested in
    val agreement_inv = r::agreement_inv

    // the inductive invariant
    val ind_inv = and {
        lemma0_committed_blocks_have_justified_qc,
        lemma1_justified_qc_is_valid,
        lemma2_qc_is_supported_by_messages,
        lemma3_signed_commit_needs_proposal,
        lemma5_signed_commit_no_equivocation,
        lemma6_signed_timeout_no_equivocation,
        lemma8_no_commits_in_future,
        lemma9_no_timeouts_in_future,
        lemma10_local_high_vote,
        lemma11_one_proposal_per_view,
        lemma12_view0_is_correct,
        lemma13_replica_and_proposer_views,
        lemma14_replica_state_high_vote_matches_sent_commit,
        lemma15_timeout_high_vote_is_supported_by_commits,
        lemma16_proposals_are_valid,
        lemma17_timeout_high_commit_qc_is_valid,
        lemma19_replica_state_high_commit_qc_is_justified,
        lemma20_replica_state_high_timeout_qc_is_justified,
    }

    // auxiliary invariants that we call lemmas

    // a committed block is supported by a CommitQC, hence, the QC should be valid
    val lemma0_committed_blocks_have_justified_qc = {
        I_CORRECT.forall(id => {
            val replica = r::replica_state.get(id)
            replica.committed_blocks.indices().forall(i => {
                val cblock = replica.committed_blocks[i]
                val qc = r::ghost_commit_qc[cblock.commit_qc]
                and {
                    qc.vote.block_hash == hash(cblock.block),
                    qc.vote.block_number == i,
                    // when the replica commits the block,
                    // it uses the CommitQC as a justification to start the new view
                    commit_qc_supported_by_messages(qc),
                    r::commit_qc_verify(qc),
                }
            })
        })
    }

    // if a correct block uses a justification, this justification should be valid
    val lemma1_justified_qc_is_valid = and {
        val commit_qc_size = r::ghost_commit_qc.length()
        val timeout_qc_size = r::ghost_timeout_qc.length()
        r::msgs_new_view.forall(m => and {
            // the view is not ill-formed
            match (m.justification) {
            | Commit(qc_id) => 0 <= qc_id and qc_id < commit_qc_size
            | Timeout(qc_id) => 0 <= qc_id and qc_id < timeout_qc_size
            },
            r::REPLICAS.contains(m.sig),
            // justifications are not ill-formed
            or {
                I_FAULTY.contains(m.sig),
                val view = r::view(m.justification)
                val id = m.sig
                and {
                    // A late replica may catch up with a CommitQC with later views)
                    // Hence, we have found that the commented condition is incorrect.
                    // view <= r::replica_state.get(id).view,

                    // (1) check the basic constraints
                    match (m.justification) {
                    | Commit(qc_id) =>
                        val qc = r::ghost_commit_qc[qc_id]
                        and {
                            qc.vote.view + 1 == view,
                            qc.agg_sig.subseteq(IREPLICAS),
                            // no invalid blocks are committed
                            I_VALID_BLOCKS.contains(qc.vote.block_hash),
                            // check the quorums in (2), as it is an expensive test
                        }

                    | Timeout(qc_id) => and {
                            val qc = r::ghost_timeout_qc[qc_id]
                            qc.ghost_view + 1 == view
                            // check the structure in (3)
                        }
                    }
                }
            }
        }),
        // (2) check the quorums in CommitQC's separately
        val commit_qc_id_in_new_view =
            r::msgs_new_view.map(m => match (m.justification) {
                | Commit(qc_id) =>
                    if (I_CORRECT.contains(m.sig)) qc_id else -1
                | _ => -1
            }).union(I_CORRECT.map(id => match (r::replica_state.get(id).high_commit_qc) {
                | Some(qc_id) => qc_id
                | None => -1
            }))
        0.to(MAX_COMMIT_QC).forall(qc_id => {
            (commit_qc_id_in_new_view.contains(qc_id))
                implies r::aux_commit_qc_inv(Some(qc_id))
        }),
        // (3) check TimeoutQC's separately
        val timeout_qc_id_in_new_view =
            r::msgs_new_view.map(m => match (m.justification) {
                | Timeout(qc_id) =>
                    if (I_CORRECT.contains(m.sig)) qc_id else -1
                | _ => -1
            }).union(I_CORRECT.map(id => match (r::replica_state.get(id).high_timeout_qc) {
                | Some(qc_id) => qc_id
                | None => -1
            }))
        0.to(MAX_TIMEOUT_QC).forall(qc_id => {
            timeout_qc_id_in_new_view.contains(qc_id) implies and {
                r::aux_timeout_qc_inv(Some(qc_id)),
            }
        }),
    }

    // we require every CommitQC and TimeoutQC to be supported by the messages
    // (except for the faulty replicas)
    val lemma2_qc_is_supported_by_messages = and {
        0.to(MAX_COMMIT_QC).forall(qc_id => or {
            qc_id >= r::ghost_commit_qc.length(),
            commit_qc_supported_by_messages(r::ghost_commit_qc[qc_id])
        }),
        0.to(MAX_TIMEOUT_QC).forall(qc_id => or {
            qc_id >= r::ghost_timeout_qc.length(),
            timeout_qc_supported_by_messages(r::ghost_timeout_qc[qc_id])
        }),
    }

    // all commits signed by correct replicas require valid proposals
    val lemma3_signed_commit_needs_proposal = and {
        // project signed commits to triples, in order to decrease the constraint set
        val triples = tuples(I_VIEWS, I_BLOCK_NUMBERS, I_VALID_BLOCKS)
            .filter(((v, bn, bh)) => r::msgs_signed_commit.exists(m => and {
                not(I_FAULTY.contains(m.sig)),
                m.vote.view == v,
                m.vote.block_number == bn,
                m.vote.block_hash == bh
            }))
        // make sure that every triple is backed by a valid proposal
        triples.forall(((v, bn, bh)) =>
            r::msgs_proposal.exists(p => and {
                p.sig == r::leader.get(v),
                p.ghost_block_number == bn,
                // the justification passes the verification
                r::justification_verify(p.justification),
                // the justification should match the commit view
                v == r::view(p.justification),
                // the commit uses get_implied_block
                val num_and_hash = r::get_implied_block(p.justification)
                and {
                    num_and_hash._1 == bn,
                    match (num_and_hash._2) {
                    | None => p.block == Some(bh)
                    | Some(b) => b == bh
                    },
                }
            })
        ),
        r::msgs_signed_commit.forall(m => or {
            I_FAULTY.contains(m.sig),
            I_VALID_BLOCKS.contains(m.vote.block_hash)
        })
    }

    val lemma5_signed_commit_no_equivocation = {
        I_CORRECT.forall(id => I_VIEWS.forall(v => {
            // use filter/map to decrease the potential set size
            val num_and_hash =
                r::msgs_signed_commit
                    .filter(m => m.sig == id and m.vote.view == v)
                    .map(m => (m.vote.block_number, m.vote.block_hash))
            val pairs = tuples(I_BLOCK_NUMBERS, IALL_BLOCKS).intersect(num_and_hash)
            pairs.forall(t1 => pairs.forall(t2 => t1 == t2))
        }))
    }

    val lemma6_signed_timeout_no_equivocation = {
        I_CORRECT.forall(id => I_VIEWS.forall(v => {
            // use filter/map to decrease the potential set size
            val triples =
                r::msgs_signed_timeout
                    .filter(m => m.sig == id and m.vote.view == v)
                    .map(m => (m.high_commit_qc, m.vote.high_vote, m.vote.high_commit_qc_view))
            triples.forall(t1 => triples.forall(t2 => t1 == t2))
        }))
    }

    val lemma8_no_commits_in_future = {
        r::msgs_signed_commit.forall(m => or {
            I_FAULTY.contains(m.sig),
            val self = r::replica_state.get(m.sig)
            or {
                m.vote.view < self.view,
                m.vote.view == self.view and self.phase != PhasePrepare,
            }
        })
    }

    val lemma9_no_timeouts_in_future = {
        r::msgs_signed_timeout.forall(m => or {
            I_FAULTY.contains(m.sig),
            val self = r::replica_state.get(m.sig)
            or {
                m.vote.view < self.view,
                m.vote.view == self.view and self.phase == PhaseTimeout,
            }
        })
    }

    // the invariant LocalHighVote from the paper
    val lemma10_local_high_vote = {
        0.to(MAX_COMMIT_QC).forall(qc_id => or {
            // the CommitQC is invalid
            qc_id >= r::ghost_commit_qc.length(),
            not(r::aux_commit_qc_inv(Some(qc_id))),
            // LocalHighVote holds true
            val qc = r::ghost_commit_qc[qc_id]
            qc.agg_sig.intersect(I_CORRECT).forall(id => {
                val self = r::replica_state.get(id)
                or {
                    self.view < qc.vote.view,
                    and {
                        self.view == qc.vote.view,
                        self.phase == PhasePrepare,
                    },
                    match (self.high_vote) {
                    | Some(hv) =>
                        or {
                            and {
                                hv.block_number == qc.vote.block_number,
                                hv.block_hash == qc.vote.block_hash,
                            },
                            hv.block_number > qc.vote.block_number,
                        }
                    | None => false
                    },
                }
            })
        })
    }

    val lemma11_one_proposal_per_view = {
        r::msgs_proposal.forall(m => or {
            I_FAULTY.contains(m.sig),
            val num_and_hash = r::get_implied_block(m.justification)
            and {
                m.ghost_block_number == num_and_hash._1,
                val qc_view = r::view(m.justification)
                and {
                    // it's only possible to send a proposal within the views window
                    I_VIEWS.contains(qc_view),
                    // the proposal is as predicted by prophecy_correct_proposals
                    match (num_and_hash._2) {
                    | Some(b) => m.block == BLOCK_NONE
                    | None => m.block == Some(r::prophecy_correct_proposals.get(qc_view))
                    },
                }
            }
        })
    }

    // synchronize the inductive invariant with init_view_1_with_leader
    val lemma12_view0_is_correct = {
        pure val init_timeout_qc: TimeoutQC = {
            votes: IREPLICAS.map(id => id).mapBy(_ => {
                view: 0,
                high_vote: COMMIT_VOTE_NONE,
                high_commit_qc_view: VIEW_NUMBER_NONE
            }),
            agg_sig: IREPLICAS.map(id => id),
            high_commit_qc: HIGH_COMMIT_QC_NONE,
            ghost_view: 0,
        }
        and {
            // the first QC is the timeout initialization
            r::ghost_timeout_qc[0] == init_timeout_qc,
            // all replicas have sent a timeout in view 0
            IREPLICAS.forall(id => r::msgs_signed_timeout.contains({
                sig: id,
                high_commit_qc: HIGH_COMMIT_QC_NONE,
                vote: { view: 0, high_vote: COMMIT_VOTE_NONE, high_commit_qc_view: VIEW_NUMBER_NONE },
            })),
            // all replicas have stored the initial timeout QC
            I_CORRECT.forall(id => {
                r::msgs_new_view.contains({
                    sig: id,
                    justification: Timeout(0)
                })
            }),
            // all replicas have sent the new_view message in view 0
            I_CORRECT.forall(id => {
                r::msgs_new_view.contains({
                    sig: id,
                    justification: Timeout(0) })
            }),
        }
    }

    val lemma13_replica_and_proposer_views = {
        I_CORRECT.forall(id =>
            val self_view = r::replica_state.get(id).view
            val replica_view = r::replica_view.get(id)
            val self_state = r::replica_state.get(id).phase
            and {
                // replica may go ahead of the proposer component
                r::proposer_view.get(id) <= self_view,
                // replica_view is used for timeouts.
                // It is not even read in the specification.
                replica_view <= self_view,
            }
        )
    }

    val lemma14_replica_state_high_vote_matches_sent_commit = {
        I_CORRECT.forall(id => 
            val self = r::replica_state.get(id)
            // all votes that were sent by the replica in on_proposal
            val all_commit_votes =
                r::msgs_signed_commit.filter(m => m.sig == id).map(m => m.vote)
            and {
                match (self.high_vote) {
                | None =>
                    all_commit_votes == Set()

                | Some(hv) =>
                    and {
                        // The high vote has the highest view among the commit votes.
                        all_commit_votes.contains(hv),
                        all_commit_votes.forall(v => (v.view <= hv.view)),
                        // The high vote is synchronized with the replica view
                        hv.view <= self.view,
                    }
                }
            }
        )
    }

    val lemma15_timeout_high_vote_is_supported_by_commits = {
        r::msgs_signed_timeout.forall(tv => or {
            I_FAULTY.contains(tv.sig),
            val all_past_commit_votes =
                r::msgs_signed_commit
                    .filter(v => v.sig == tv.sig and v.vote.view <= tv.vote.view)
            match (tv.vote.high_vote) {
            | None =>
                all_past_commit_votes == Set()
            | Some(hv) =>
                // the high vote has the highest view among the commit votes,
                // if they do exist
                val max_view =
                    all_past_commit_votes.map(v => v.vote.view)
                        .fold(-1, (i, j) => if (i > j) i else j)
                and {
                    // if max_view == -1, then tv.vote.high_vote == None
                    max_view >= 0,
                    // the maximal commit vote is the high vote in the timeout vote
                    all_past_commit_votes.forall(v => {
                        (v.vote.view == max_view) implies (hv == v.vote)
                    })
                }
            },
        })
    }

    // all proposals sent by correct replicas are valid
    val lemma16_proposals_are_valid = {
        r::msgs_proposal.forall(m => and {
            I_CORRECT.contains(m.sig) implies r::proposal_verify(m),
            // even if a proposal is signed by a faulty replica,
            // the signatures in CommitQC must be supported
            justification_is_supported_with_messages(m.justification),
        })
    }

    val lemma17_timeout_high_commit_qc_is_valid = {
        val commit_qc_size = r::ghost_commit_qc.length()
        r::msgs_signed_timeout.forall(tv => and {
            match (tv.high_commit_qc) {
            | Some(qc_id) => (0 <= qc_id and qc_id < commit_qc_size)
            | None => true
            },
            or {
                I_FAULTY.contains(tv.sig),
                r::aux_commit_qc_inv(tv.high_commit_qc)
            }
        })
    }

    val lemma19_replica_state_high_commit_qc_is_justified = {
        I_CORRECT.forall(id => 
            val self = r::replica_state.get(id)
            and {
                match (self.high_commit_qc) {
                | None =>
                    true
                | Some(qc_id) =>
                    commit_qc_supported_by_messages(r::ghost_commit_qc[qc_id])
                },
            }
        )
    }

    val lemma20_replica_state_high_timeout_qc_is_justified = {
        I_CORRECT.forall(id => 
            val self = r::replica_state.get(id)
            and {
                match (self.high_timeout_qc) {
                | None =>
                    true // this is not possible in views above zero

                | Some(qc_id) =>
                    timeout_qc_supported_by_messages(r::ghost_timeout_qc[qc_id])
                },
            }
        )
    }

    // HighVoteInTimeoutQC in the paper
    val lemma21_high_vote_in_timeout_qc = {
        0.to(MAX_COMMIT_QC).forall(cqc_id => or {
            cqc_id >= r::ghost_commit_qc.length(),
            not(r::aux_commit_qc_inv(Some(cqc_id))),
            0.to(MAX_TIMEOUT_QC).forall(tqc_id => or {
                tqc_id >= r::ghost_timeout_qc.length(),
                not(r::aux_timeout_qc_inv(Some(tqc_id))),
                val cqc = r::ghost_commit_qc[cqc_id]
                val tqc = r::ghost_timeout_qc[tqc_id]
                and {
                    match (tqc.high_commit_qc) {
                    | Some(hqc_id) =>
                        val hqc = r::ghost_commit_qc[hqc_id]
                        hqc.vote.block_number == cqc.vote.block_number - 1
                    | None => false
                    },
                    tqc.ghost_view >= cqc.vote.view,
                } implies {
                    match (r::high_vote(tqc)) {
                    | Some(hv) => and {
                            hv.block_number == cqc.vote.block_number,
                            hv.block_hash == cqc.vote.block_hash,
                        }
                    | None => false
                    }
                }
            })
        })
    }

    // additional definitions
    def justification_is_supported_with_messages(j: Justification): bool = {
        match (j) {
        | Commit(qc_id) =>
            commit_qc_supported_by_messages(r::ghost_commit_qc[qc_id])

        | Timeout(qc_id) =>
            timeout_qc_supported_by_messages(r::ghost_timeout_qc[qc_id])
        }
    }

    def commit_qc_supported_by_messages(qc: CommitQC): bool = {
        qc.agg_sig.forall(id => or {
            I_FAULTY.contains(id),
            r::msgs_signed_commit.contains({ vote: qc.vote, sig: id })
        })
    }

    def timeout_qc_supported_by_messages(qc: TimeoutQC): bool = {
        qc.votes.keys().forall(id => or {
            I_FAULTY.contains(id),
            val vote = qc.votes.get(id)
            r::msgs_signed_timeout.exists(v => v.vote == vote and v.sig == id)
        })
    }

    // liveness-related reasoning

    // a magic view is the view where a lot of good things have happened
    val is_magic_view = and {
        // view 0 is simply bootstrapping
        prophecy_magic_view > 0,
        // the leader in the magic view is correct
        I_CORRECT.contains(r::leader.get(prophecy_magic_view)),
        // all correct replicas have progressed beyond the magic view
        I_CORRECT.forall(id => r::replica_view.get(id) > prophecy_magic_view),
        // no correct replica has sent a timeout in the magic view
        I_CORRECT.forall(id => r::msgs_signed_timeout.forall(m => or {
            m.sig != id,
            m.vote.view != prophecy_magic_view
        })),
    }

    // check this property to get an example
    val magic_view_example = not(is_magic_view)

    // if there was a magic round, then at least one correct replica committed a block
    val magic_view_implies_decision = {
        is_magic_view implies
            // TODO: say that the block was committed in the magic_view
            I_CORRECT.exists(id => {
                val replica = r::replica_state.get(id)
                length(replica.committed_blocks) != 0
            })
            // TODO: all replicas have the QC?
    }

    // check:
    // JVM_ARGS=-Xmx20G quint verify --max-steps=1 --server-endpoint=localhost:20001 \
    // --apalache-config=apalache-inductive.json \
    // --init=ind_init --step=post_ind_inv \
    // --invariant=magic_implies_decision n6f1b1_inductive.qnt

    // auxiliary definitions

    action unchanged_all = all {
        // keep everything unchanged
        r::leader' = r::leader,
        r::replica_view' = r::replica_view,
        r::proposer_view' = r::proposer_view,
        r::replica_state' = r::replica_state,
        r::ghost_commit_qc' = r::ghost_commit_qc,
        r::ghost_timeout_qc' = r::ghost_timeout_qc,
        r::msgs_signed_commit' = r::msgs_signed_commit,
        r::msgs_signed_timeout' = r::msgs_signed_timeout,
        r::msgs_new_view' = r::msgs_new_view,
        r::msgs_proposal' = r::msgs_proposal,
        r::ghost_justifications' = r::ghost_justifications,
        r::ghost_step' = r::ghost_step,
        r::prophecy_correct_proposals' = r::prophecy_correct_proposals,
        prophecy_magic_view' = prophecy_magic_view,
        step_no' = step_no + 1,
    }

    val shape_inv = and {
        r::ghost_commit_qc.indices().forall(i => commit_qc_shape_inv(r::ghost_commit_qc[i])),
        r::ghost_timeout_qc.indices().forall(i => timeout_qc_shape_inv(r::ghost_timeout_qc[i], r::ghost_commit_qc.length())),
        r::msgs_signed_commit.forall(signed_commit_vote_shape_inv),
        r::msgs_signed_timeout.forall(signed_timeout_vote_shape_inv),
    }

    val one_block_example = r::one_block_example

    val phase_commit_example = r::phase_commit_example

    val msgs_signed_commit_example = r::msgs_signed_commit == Set()

    val high_timeout_qc_example = r::high_timeout_qc_example
    
    val commit_qc_example = r::commit_qc_example

    val ghost_commit_qc_example = r::ghost_commit_qc.length() == 0

    val ghost_step_commit_example = r::ghost_step != OnCommitStep("n0")
}