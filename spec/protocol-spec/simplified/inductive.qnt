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
module inductive {
    import types.* from "./types"
    import option.* from "../option"
    
    // parameters
    const IN: int
    const IF: int
    const ICORRECT: Set[ReplicaId]
    const IFAULTY: Set[ReplicaId]
    const IVIEWS: Set[ViewNumber]
    const IBLOCK_NUMBERS: Set[BlockNumber]
    const IVALID_BLOCKS: Set[Block]
    const IINVALID_BLOCKS: Set[Block]

    // we restrict the number of CommitQC and TimeoutQC. Otherwise, the search space is too large.
    const MAX_COMMIT_QC: int
    const MAX_TIMEOUT_QC: int

    pure val IALL_BLOCKS = IVALID_BLOCKS.union(IINVALID_BLOCKS)
    pure val IREPLICAS = ICORRECT.union(IFAULTY)

    import replica(
        N = IN,
        F = IF,
        CORRECT = ICORRECT,
        FAULTY = IFAULTY,
        VIEWS = IVIEWS,
        VALID_BLOCKS = IVALID_BLOCKS,
        INVALID_BLOCKS = IINVALID_BLOCKS
    ) as r from "./replica"

    var step_no: int
    // if prophecy_magic_view > 0,
    // a number of good conditions have been met in prophecy_magic_view (see below)
    var prophecy_magic_view: int

    // definitions

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
        match (v.high_vote) {
        | Some(hv) => commit_vote_shape_inv(hv)
        | None => true
        },
        match (v.high_commit_qc_view) {
        | Some(i) => IVIEWS.contains(i)
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
        IVIEWS.contains(qc.ghost_view),
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
        tuples(IVIEWS.exclude(Set(0)), IBLOCK_NUMBERS, IALL_BLOCKS)
            .map(((v, bn, bh)) => {
                { view: v, block_number: bn, block_hash: bh }
            })

    pure val SIGNED_COMMIT_VOTES: Set[SignedCommitVote] =
        tuples(IVIEWS.exclude(Set(0)), IBLOCK_NUMBERS, IALL_BLOCKS, IREPLICAS)
            .map(((v, bn, bh, sig)) => {
                { vote: { view: v, block_number: bn, block_hash: bh }, sig: sig }
            })

    pure val COMMIT_QCS: Set[CommitQC] = tuples(COMMIT_VOTES, powerset(IREPLICAS)).map(((cv, sigs)) => {
        { vote: cv, agg_sig: sigs }
    })

    pure val COMMIT_VOTES_OR_NONE: Set[Option[CommitVote]] = COMMIT_VOTES.map(v => Some(v)).union(Set(COMMIT_VOTE_NONE))

    pure val VIEW_NUMBERS_OR_NONE: Set[Option[ViewNumber]] = IVIEWS.map(v => Some(v)).union(Set(VIEW_NUMBER_NONE))

    pure val TIMEOUT_VOTES: Set[TimeoutVote] =
        tuples(IVIEWS, COMMIT_VOTES_OR_NONE, VIEW_NUMBERS_OR_NONE).map(((v, hv, hcv)) =>
            { view: v, high_vote: hv, high_commit_qc_view: hcv }
        )

    pure val SIGNED_TIMEOUT_VOTES: Set[SignedTimeoutVote] =
        tuples(IVIEWS, COMMIT_VOTES_OR_NONE, VIEW_NUMBERS_OR_NONE, IREPLICAS, 0.to(MAX_COMMIT_QC), Bool)
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

    pure val PROPOSALS: Set[Proposal] =
        tuples(IALL_BLOCKS, Bool, 0.to(MAX_COMMIT_QC).union(0.to(MAX_TIMEOUT_QC)), Bool, IREPLICAS, IBLOCK_NUMBERS)
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
        IBLOCK_NUMBERS.contains(sv.ghost_block_number),
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
        tuples(IBLOCK_NUMBERS, IALL_BLOCKS).powerset()

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
        nondet mv = IVIEWS.oneOf()
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
        nondet timeout_qc_view = 0.to(MAX_TIMEOUT_QC).setOfMaps(IVIEWS).oneOf()
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
        // non-deterministically initialize msgs_timeout_commit
        //nondet signed_timeouts = SIGNED_TIMEOUT_VOTES.powerset().oneOf()
        // non-deterministically initialize msgs_new_view
        nondet new_views = NEW_VIEWS.powerset().oneOf()
        // non-deterministically initialize msgs_proposal
        //nondet proposals = PROPOSALS.powerset().oneOf()
        // non-deterministically initialize ghost_justifications
        nondet justifications =
            tuples(ICORRECT, IVIEWS.exclude(Set(0))).setOfMaps(JUSTIFICATIONS).oneOf()
        // Non-deterministically initialize replica_state.
        // We pick maps for the individual fields, to avoid combinatorial explosion of tuples.
        nondet state_views = ICORRECT.setOfMaps(IVIEWS).oneOf()
        nondet state_phases = ICORRECT.setOfMaps(PHASES).oneOf()
        nondet high_votes = ICORRECT.setOfMaps(COMMIT_VOTES_OR_NONE).oneOf()
        nondet high_commit_qcs = ICORRECT.setOfMaps(COMMIT_QC_ID_OR_NONE).oneOf()
        nondet high_timeout_qcs = ICORRECT.setOfMaps(TIMEOUT_QC_ID_OR_NONE).oneOf()
        nondet cached_proposals = ICORRECT.setOfMaps(CACHED_PROPOSALS).oneOf()
        nondet committed_blocks = ICORRECT.setOfMaps(COMMITTED_BLOCKS_0_OR_1).oneOf()
        val replica_s: ReplicaId -> ReplicaState =
            ICORRECT.mapBy(id =>
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
        nondet ldr = IVIEWS.setOfMaps(IREPLICAS).oneOf()
        // non-deterministically choose replica_view and proposer_view
        nondet rv = ICORRECT.setOfMaps(IVIEWS).oneOf()
        nondet pv = ICORRECT.setOfMaps(IVIEWS).oneOf()
        // guess prophecy_correct_proposals
        nondet i_prophecy_correct_proposals = IVIEWS.setOfMaps(IVALID_BLOCKS).oneOf()
        all {
            r::leader' = ldr,
            r::replica_view' = rv,
            r::proposer_view' = pv,
            r::replica_state' = replica_s,
            r::ghost_commit_qc' = commit_qc,
            r::ghost_timeout_qc' = timeout_qc,
            r::msgs_signed_commit' = signed_commits,
            //r::msgs_signed_timeout' = signed_timeouts,
            {
                // we have to pass the upper bound of 10 directly (this is a limitation of Quint)
                nondet msgs_signed_timeout = generate(10, Set(SIGNED_TIMEOUT_VOTE_SAMPLE))
                all {
                    msgs_signed_timeout.forall(signed_timeout_vote_shape_inv),
                    r::msgs_signed_timeout' = msgs_signed_timeout,
                }
            },
            r::msgs_new_view' = new_views,
            //r::msgs_proposal' = proposals,
            {
                // we have to pass the upper bound of 10 directly (this is a limitation of Quint)
                nondet msgs_proposal = generate(10, Set(PROPOSAL_SAMPLE))
                all {
                    msgs_proposal.forall(proposal_shape_inv),
                    r::msgs_proposal' = msgs_proposal,
                }
            },
            r::ghost_justifications' = justifications,
            r::ghost_step' = AnyStep,
            r::prophecy_correct_proposals' = i_prophecy_correct_proposals,
            nondet mv = IVIEWS.oneOf()
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
        lemma2_justified_commit_qc_supported_by_messages,
        lemma3_signed_commit_needs_proposal,
        lemma4_signed_commit_needs_justification,
        lemma5_signed_commit_no_equivocation,
        lemma6_signed_timeout_no_equivocation,
        // lemma7 is incorrect
        lemma8_no_commits_in_future,
        lemma9_no_timeouts_in_future,
        lemma10_justified_commit_qc_no_vote_change,
        lemma11_one_proposal_per_view,
        lemma12_view0_is_correct,
        lemma13_replica_and_proposer_views,
        lemma14_replica_state_high_vote_matches_sent_commit,
        lemma15_timeout_high_vote_is_supported_by_commits,
        lemma16_proposals_are_valid,
    }

    // auxiliary invariants that we call lemmas

    // a committed block is supported by a CommitQC, hence, the QC should be valid
    val lemma0_committed_blocks_have_justified_qc = {
        ICORRECT.forall(id => {
            val replica = r::replica_state.get(id)
            replica.committed_blocks.indices().forall(i => {
                val cblock = replica.committed_blocks[i]
                val qc = r::ghost_commit_qc[cblock.commit_qc]
                and {
                    qc.vote.block_hash == hash(cblock.block),
                    qc.vote.block_number == i,
                    // when the replica commits the block,
                    // it uses the CommitQC as a justification to start the new view
                    r::ghost_justifications.keys().contains((id, qc.vote.view + 1)),
                    r::ghost_justifications.get((id, qc.vote.view + 1)) == Commit(cblock.commit_qc),
                }
            })
        })
    }

    // if a correct block uses a justification, this justification should be valid
    val lemma1_justified_qc_is_valid = and {
        r::ghost_justifications.keys().forall(((id, view)) => {
            // (1) check the basic constraints
            match (r::ghost_justifications.get((id, view))) {
            | Commit(qc_id) =>
                val qc = r::ghost_commit_qc[qc_id]
                and {
                    qc.vote.view + 1 == view,
                    qc.agg_sig.subseteq(IREPLICAS),
                    // no invalid blocks are committed
                    IVALID_BLOCKS.exists(b => hash(b) == qc.vote.block_hash),
                    // check the quorums in (2), as it is an expensive test
                }

            | Timeout(qc_id) =>
                val qc = r::ghost_timeout_qc[qc_id]
                qc.ghost_view + 1 == view
                // check the structure in (3)
            }
        }),
        // (2) check the quorums in CommitQC's separately
        val commit_qc_id_in_justifications =
            r::ghost_justifications.keys()
                .map(((id, view)) => match (r::ghost_justifications.get((id, view))) {
                | Commit(qc_id) => qc_id
                | _ => -1
            })
        0.to(MAX_COMMIT_QC).forall(qc_id => {
            (commit_qc_id_in_justifications.contains(qc_id))
                implies r::commit_qc_verify(r::ghost_commit_qc[qc_id])
        }),
        // (3) check TimeoutQC's separately
        val timeout_qc_id_in_justifications =
            r::ghost_justifications.keys()
                .map(((id, view)) => match (r::ghost_justifications.get(((id, view)))) {
                | Timeout(qc_id) => qc_id
                | _ => -1
            })
        0.to(MAX_TIMEOUT_QC).forall(qc_id => {
            timeout_qc_id_in_justifications.contains(qc_id) implies and {
                r::aux_timeout_qc_inv(Some(qc_id)),
            }
        }),
    }

    val lemma2_justified_commit_qc_supported_by_messages = {
        r::ghost_justifications.keys().forall(((id, view)) => {
            match (r::ghost_justifications.get((id, view))) {
            | Commit(qc_id) =>
                val qc = r::ghost_commit_qc[qc_id]
                qc.agg_sig.forall(id =>
                    r::msgs_signed_commit.contains({ vote: qc.vote, sig: id })
                )

            | Timeout(qc_id) =>
                // TODO
                true
            }
        })
    }

    val lemma3_signed_commit_needs_proposal = {
        r::msgs_signed_commit.forall(m => or {
            IFAULTY.contains(m.sig),
            0.to(MAX_COMMIT_QC).exists(qc_id => {
                val proposal = {
                    // in our modeling, the block hash is identical to the block
                    block: Some(m.vote.block_hash),
                    justification: Commit(qc_id),
                    sig: r::leader.get(m.vote.view),
                    ghost_block_number: m.vote.block_number - 1,
                }
                r::msgs_proposal.contains(proposal)
            }),
            0.to(MAX_TIMEOUT_QC).exists(qc_id => and {
                val proposal = {
                    block: Some(m.vote.block_hash),
                    justification: Timeout(qc_id),
                    sig: r::leader.get(m.vote.view),
                    ghost_block_number: m.vote.block_number,
                }
                r::msgs_proposal.contains(proposal),
            }),
        })
    }

    val lemma4_signed_commit_needs_justification = {
        r::ghost_justifications.keys().forall(((id, view)) => {
            val justification = r::ghost_justifications.get((id, view))
            val num_and_hash = r::get_implied_block(justification)
            r::msgs_signed_commit.forall(m => or {
                IFAULTY.contains(m.sig),
                m.sig != id,
                m.vote.view != view,
                and {
                    num_and_hash._1 == m.vote.block_number,
                    match (num_and_hash._2) {
                    | None => true
                    | Some(b) => b == m.vote.block_hash
                    },
                }
            })
        })
    }

    val lemma5_signed_commit_no_equivocation = {
        ICORRECT.forall(id => IVIEWS.forall(v => {
            // use filter/map to decrease the potential set size
            val num_and_hash =
                r::msgs_signed_commit
                    .filter(m => m.sig == id and m.vote.view == v)
                    .map(m => (m.vote.block_number, m.vote.block_hash))
            val pairs = tuples(IBLOCK_NUMBERS, IALL_BLOCKS).intersect(num_and_hash)
            pairs.forall(t1 => pairs.forall(t2 => t1 == t2))
        }))
    }

    val lemma6_signed_timeout_no_equivocation = {
        ICORRECT.forall(id => IVIEWS.forall(v => {
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
            IFAULTY.contains(m.sig),
            val self = r::replica_state.get(m.sig)
            or {
                m.vote.view < self.view,
                m.vote.view == self.view and self.phase != PhasePrepare,
            }
        })
    }

    val lemma9_no_timeouts_in_future = {
        r::msgs_signed_timeout.forall(m => or {
            IFAULTY.contains(m.sig),
            val self = r::replica_state.get(m.sig)
            or {
                m.vote.view < self.view,
                m.vote.view == self.view and self.phase == PhaseTimeout,
            }
        })
    }

    // once a correct replica commits to a block (as a part of QC),
    // it will never change its mind
    val lemma10_justified_commit_qc_no_vote_change = {
        r::ghost_justifications.keys().forall(((id, view)) => {
            match (r::ghost_justifications.get((id, view))) {
            | Commit(qc_id) =>
                val qc = r::ghost_commit_qc[qc_id]
                r::msgs_signed_commit.forall(m => {
                    and {
                        ICORRECT.contains(m.sig),
                        qc.agg_sig.contains(m.sig),
                        m.vote.view >= qc.vote.view,
                        m.vote.block_number == qc.vote.block_number
                    } implies (m.vote.block_hash == qc.vote.block_hash)
                })

            | Timeout(qc_id) =>
                true
            }
        })
    }

    val lemma11_one_proposal_per_view = {
        r::msgs_proposal.forall(m => or {
            IFAULTY.contains(m.sig),
            val num_and_hash = r::get_implied_block(m.justification)
            and {
                m.ghost_block_number == num_and_hash._1,
                val qc_view = r::view(m.justification)
                match (num_and_hash._2) {
                | Some(b) => m.block == BLOCK_NONE
                | None => m.block == Some(r::prophecy_correct_proposals.get(qc_view))
                },
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
            ICORRECT.forall(id => {
                r::ghost_justifications.get((id, 1)) == Timeout(0)
            }),
            // all replicas have sent the new_view message in view 0
            ICORRECT.forall(id => {
                r::msgs_new_view.contains({
                    sig: id,
                    justification: Timeout(0) })
            }),
        }
    }

    val lemma13_replica_and_proposer_views = {
        ICORRECT.forall(id =>
            val self_view = r::replica_state.get(id).view
            val replica_view = r::replica_view.get(id)
            and {
                r::proposer_view.get(id) <= self_view,
                or {
                    replica_view == self_view,
                    replica_view + 1 == self_view,
                }
            }
        )
    }

    val lemma14_replica_state_high_vote_matches_sent_commit = {
        ICORRECT.forall(id => 
            val self = r::replica_state.get(id)
            // all votes that were sent by the replica in on_proposal
            val all_commit_votes =
                r::msgs_signed_commit.filter(m => m.sig == id).map(m => m.vote)
            and {
                match (self.high_vote) {
                | None => all_commit_votes == Set()
                | Some(hv) =>
                    and {
                        all_commit_votes != Set(),
                        // the high vote has the highest view among the commit votes
                        all_commit_votes.forall(v => and {
                            v.view <= hv.view,
                            v.view == hv.view implies hv == v,
                        })
                    }
                }
            }
        )
    }

    val lemma15_timeout_high_vote_is_supported_by_commits = {
        r::msgs_signed_timeout.forall(tv => or {
            IFAULTY.contains(tv.sig),
            val all_past_commit_votes =
                r::msgs_signed_commit
                    .filter(v => v.sig == tv.sig and v.vote.view <= tv.vote.view)
            match (tv.vote.high_vote) {
            | None =>
                all_past_commit_votes == Set()
            | Some(hv) =>
                // the high vote has the highest view among the commit votes
                val max_view =
                    all_past_commit_votes.map(v => v.vote.view)
                        .fold(0, (i, j) => if (i > j) i else j)
                all_past_commit_votes.forall(v => {
                    (v.vote.view == max_view) implies (hv == v.vote)
                })
            },
        })
    }

    // all proposals sent by correct replicas are valid
    val lemma16_proposals_are_valid = {
        r::msgs_proposal.forall(m => or {
            IFAULTY.contains(m.sig),
            r::proposal_verify(m),
        })
    }

    // invalid lemmas

    // this lemma actually does not hold true, we have a counterexample
    val lemma7_proposal_no_equivocation = {
        r::msgs_proposal.forall(m1 => or {
            IFAULTY.contains(m1.sig),
            r::msgs_proposal.forall(m2 => {
                (m1.sig == m2.sig and m1.ghost_block_number == m2.ghost_block_number)
                    implies (m1 == m2)
            })
        })
    }

    // liveness-related reasoning

    // a magic view is the view where a lot of good things have happened
    val is_magic_view = and {
        // view 0 is simply bootstrapping
        prophecy_magic_view > 0,
        // the leader in the magic view is correct
        ICORRECT.contains(r::leader.get(prophecy_magic_view)),
        // all correct replicas have progressed beyond the magic view
        ICORRECT.forall(id => r::replica_view.get(id) > prophecy_magic_view),
        // no correct replica has sent a timeout in the magic view
        ICORRECT.forall(id => r::msgs_signed_timeout.forall(m => or {
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
            ICORRECT.exists(id => {
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