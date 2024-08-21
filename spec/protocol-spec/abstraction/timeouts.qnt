// A snippet of ../replica.qnt that is heavily optimized for inductive verification.
//
// quint verify --apalache-config=apalache-inductive.json --max-steps=1 --init=init_ind --step=step_ind --invariant=inv n6f1b1.qnt
// ...
// 18566ms
module timeouts {
    import option.* from "../option"

    type ReplicaId = str
    type ReplicaKey = str
    type ViewNumber = int
    type Block = str
    type Signature = str
    type AggregateSignature = Set[Signature]

    type Phase = PhasePrepare | PhaseTimeout

    type ReplicaState = {
        view: ViewNumber,
        phase: Phase,
        high_timeout_qc: Option[TimeoutQC],
    }

    // If we have timeout votes with at least QUORUM_WEIGHT for the same
    // view from different replicas, we can create a timeout quorum certificate
    // locally. Contrary to the commit QC, this QC aggregates different messages.
    type TimeoutQC = {
        votes: ReplicaKey -> TimeoutVote,
        agg_sig: AggregateSignature,
        ghost_view: ViewNumber,
    }

    type TimeoutVote = {
        view: ViewNumber,
    }

    type SignedTimeoutVote = {
        vote: TimeoutVote,
        sig: Signature,
    }

    type NewView = {
        justification: TimeoutQC,
        sig: Signature,
    }

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

    // The set of all views.
    const VIEWS: Set[ViewNumber]

    // the set of all blocks
    const VALID_BLOCKS: Set[Block]
    const INVALID_BLOCKS: Set[Block]
    pure val ALL_BLOCKS = VALID_BLOCKS.union(INVALID_BLOCKS)

    pure val HIGH_TIMEOUT_QC_NONE: Option[TimeoutQC] = None
    pure val VIEW_NUMBER_NONE: Option[ViewNumber] = None

    pure val FAULTY_WEIGHT = (N - 1) / 5;
    pure val QUORUM_WEIGHT = N - FAULTY_WEIGHT;
    pure val SUBQUORUM_WEIGHT = N - 3 * FAULTY_WEIGHT;

    pure val TIMEOUT_VOTE_SAMPLE: TimeoutVote = {
        view: 0,
    }

    pure val TIMEOUT_QC_SAMPLE: TimeoutQC = {
        votes: Set("n0").mapBy(id => TIMEOUT_VOTE_SAMPLE),
        agg_sig: Set("n0"),
        ghost_view: 0,
    }

    pure val SIGNED_TIMEOUT_VOTE_SAMPLE: SignedTimeoutVote = {
        sig: "n0",
        vote: TIMEOUT_VOTE_SAMPLE,
    }

    pure val VIEW_SAMPLE: NewView = {
        justification: TIMEOUT_QC_SAMPLE,
        sig: "n0",
    }

    pure def timeout_vote_shape_inv(v: TimeoutVote): bool = and {
        VIEWS.contains(v.view),
    }

    pure def timeout_qc_shape_inv(qc: TimeoutQC): bool = and {
        qc.votes.keys().forall(id => and {
            REPLICAS.contains(id),
            timeout_vote_shape_inv(qc.votes.get(id)),
        }),
        qc.agg_sig.subseteq(REPLICAS),
        VIEWS.contains(qc.ghost_view),
    }

    pure def signed_timeout_vote_shape_inv(sv: SignedTimeoutVote): bool = and {
        REPLICAS.contains(sv.sig),
        timeout_vote_shape_inv(sv.vote),
    }

    pure def new_view_shape_inv(new_view: NewView): bool = and {
        REPLICAS.contains(new_view.sig),
        timeout_qc_shape_inv(new_view.justification),
    }

    var replica_state: ReplicaId -> ReplicaState
    var msgs_signed_timeout: Set[SignedTimeoutVote]
    var msgs_new_view: Set[NewView]
    var store_signed_timeout: ReplicaId -> Set[SignedTimeoutVote]

    action init = all {
        replica_state' = CORRECT.mapBy(id => {
            view: 0,
            high_timeout_qc: HIGH_TIMEOUT_QC_NONE,
            phase: PhasePrepare,
        }),
        msgs_signed_timeout' = Set(),
        store_signed_timeout' = CORRECT.mapBy(id => Set()),
        msgs_new_view' = Set(),
    }

    action init_ind: bool = {
        // assign indices to all replicas in some order
        pure val idx_and_size = CORRECT.fold((Map(), 0), (p, r) => (p._1.put(r, p._2), p._2 + 1))
        pure val replica_idx = idx_and_size._1
        pure val correct_size = idx_and_size._2
        pure val all_indices = replica_idx.keys().map(id => replica_idx.get(id))
        all {
            // non-deterministically choose replica_state
            nondet replica_views = CORRECT.setOfMaps(VIEWS).oneOf()
            nondet replica_phases =
                CORRECT.setOfMaps(Set(PhasePrepare, PhaseTimeout)).oneOf()
            nondet high_timeout_qcs = generate(6, [TIMEOUT_QC_SAMPLE])
            nondet high_timeout_qcs_is_some = REPLICAS.powerset().oneOf()
            all {
                high_timeout_qcs.length() == N,
                all_indices.forall(i => timeout_qc_shape_inv(high_timeout_qcs[i])),
                replica_state' = CORRECT.mapBy(id =>
                    val i = replica_idx.get(id)
                    {
                        view: replica_views.get(id),
                        phase: replica_phases.get(id),
                        high_timeout_qc:
                            if (high_timeout_qcs_is_some.contains(id)) {
                                Some(high_timeout_qcs[i])
                            } else {
                                HIGH_TIMEOUT_QC_NONE
                            },
                    }
                ),
            },
            // non-deterministically choose msgs_signed_timeout
            {
                nondet init_msgs_signed_timeout = generate(20, Set(SIGNED_TIMEOUT_VOTE_SAMPLE))
                all {
                    init_msgs_signed_timeout.forall(signed_timeout_vote_shape_inv),
                    msgs_signed_timeout' = init_msgs_signed_timeout,
                }
            },
            // non-deterministically choose msgs_new_view
            {
                nondet init_msgs_new_view = generate(10, Set(VIEW_SAMPLE))
                all {
                    init_msgs_new_view.forall(new_view_shape_inv),
                    msgs_new_view' = init_msgs_new_view,
                }
            },
            // non-deterministically choose store_signed_timeout
            {
                nondet init_store_signed_timeout = generate(6, [Set(SIGNED_TIMEOUT_VOTE_SAMPLE)])
                all {
                    init_store_signed_timeout.length() == N,
                    init_store_signed_timeout.indices().forall(i => init_store_signed_timeout[i].forall(signed_timeout_vote_shape_inv)),
                    store_signed_timeout' = CORRECT.mapBy(id => init_store_signed_timeout[replica_idx.get(id)]),
                }
            },
        }
    }

    action step = {
        nondet id = CORRECT.oneOf()
        any {
            on_timer_is_finished(id),
            all {
                msgs_signed_timeout != Set(),
                nondet vote = oneOf(msgs_signed_timeout)
                on_timeout(id, vote),
            },
            all {
                msgs_new_view != Set(),
                nondet new_view = oneOf(msgs_new_view)
                on_new_view(id, new_view),
            }
        }
    }

    action inv = and {
        store_signed_timeout_all_inv,
        msgs_signed_timeout_inv
    }

    action step_ind = all {
        inv,
        step
    }

    action on_timer_is_finished(id: ReplicaId): bool = {
        val self = replica_state.get(id)
        val vote: SignedTimeoutVote = {
            vote: {
                view: self.view,
            },
            sig: id,
        }
        all {
            // If we have already timed out, we don't need to send another timeout vote.
            self.phase != PhaseTimeout,
            replica_state' = replica_state.set(id, { ...self, phase: PhaseTimeout }),
            msgs_signed_timeout' = msgs_signed_timeout.union(Set(vote)),
            // unchanged variables
            msgs_new_view' = msgs_new_view,
            store_signed_timeout' = store_signed_timeout,
        }
    }

    action on_timeout(id: ReplicaId, signed_vote: SignedTimeoutVote): bool = all {
        val self = replica_state.get(id)
        all {
            // If the vote isn't current, just ignore it
            signed_vote.vote.view >= self.view,
            // Check that the signed vote is valid.
            signed_vote.signed_timeout_vote_verify(),
            // Store the vote. We will never store duplicate (same view and sender) votes.
            is_new_signed_timeout_vote_for_replica(id, signed_vote),
            val new_store = store_signed_timeout.get(id).union(Set(signed_vote))

            all {
                store_signed_timeout' = store_signed_timeout.set(id, new_store),
                // Check if we now have a timeout QC for this view.
                val qc_opt = get_timeout_qc(new_store, signed_vote.vote.view)
                match (qc_opt) {
                    | None => all {
                        replica_state' = replica_state,
                        msgs_new_view' = msgs_new_view,
                    }

                    | Some(qc) => all {
                        val self1 = {
                            ...self,
                            high_timeout_qc: max_timeout_qc(qc_opt, self.high_timeout_qc)
                        }
                        start_new_view(id, self1, signed_vote.vote.view + 1, qc),
                    }
                },
                // unchanged variables
                msgs_signed_timeout' = msgs_signed_timeout,
            }
        }
    }

    action on_new_view(id: ReplicaId, new_view: NewView): bool = all {
        val self = replica_state.get(id)
        val new_view_view = new_view.justification.ghost_view
        all {
            // If the message isn't current, just ignore it.
            new_view_view >= self.view,
            // Check that the new view is valid.
            new_view.justification.timeout_qc_verify(),
            // Update our state.
            val self1 =
                {
                        ...self,
                        high_timeout_qc: max_timeout_qc(Some(new_view.justification), self.high_timeout_qc)
                }

            if (new_view_view > self.view) {
                start_new_view(id, self1, new_view_view, new_view.justification)
            } else {
                all {
                    replica_state' = replica_state,
                    msgs_new_view' = msgs_new_view,
                }
            },
            // unchanged variables
            msgs_signed_timeout' = msgs_signed_timeout,
            store_signed_timeout' = store_signed_timeout,
        }
    }

    action start_new_view(id: ReplicaId, self: ReplicaState, view: ViewNumber, justification: TimeoutQC): bool = {
        // make sure that we do not go out of bounds with the new view
        all {
            VIEWS.contains(view),
            // switch to the Prepare phase
            replica_state' = replica_state.set(id, { ...self, view: view, phase: PhasePrepare }),
            // Send a new view message to the other replicas for synchronization
            msgs_new_view' = msgs_new_view.union(Set({
                justification: justification,
                sig: id,
            })),
        }
    }

    pure def aggregate_verify(agg_sig: AggregateSignature): bool = and {
        agg_sig.size() >= QUORUM_WEIGHT,
        agg_sig.subseteq(REPLICAS),
    }

    pure def signed_timeout_vote_verify(signed_vote: SignedTimeoutVote): bool = all {
        signed_vote.vote.view >= 0,
        timeout_vote_verify(signed_vote.vote)
    }

    pure def timeout_vote_verify(vote: TimeoutVote): bool = all {
        vote.view >= 0,
        // ...
    }

    def is_new_signed_timeout_vote_for_replica(id: ReplicaId, signed_vote: SignedTimeoutVote):bool = {
        store_signed_timeout.get(id).forall(v => or {
            v.vote.view != signed_vote.vote.view,
            v.sig != signed_vote.sig
        })
    }

    def get_timeout_qc(votes: Set[SignedTimeoutVote], view: ViewNumber): Option[TimeoutQC] = {
        val view_signed_votes = votes.filter(v => v.vote.view == view)
        if (view_signed_votes.size() < QUORUM_WEIGHT) {
            None
        } else {
            // Importantly, view_signed_votes does not have two votes with the same signature.
            // We check that in store_signed_timeout_all_inv.
            val qc_votes: ReplicaKey -> TimeoutVote =
                view_signed_votes.map(v => (v.sig, v.vote)).setToMap()
            Some({
                votes: qc_votes,
                agg_sig: view_signed_votes.map(v => v.sig),
                ghost_view: view,
            })
        }
    }

    def max_timeout_qc(qc1_opt: Option[TimeoutQC], qc2_opt: Option[TimeoutQC]): Option[TimeoutQC] = {
        match (qc2_opt) {
            | None => qc1_opt
            | Some(qc2) =>
                match (qc1_opt) {
                    | None => qc2_opt
                    | Some(qc1) =>
                        if (qc1.ghost_view >= qc2.ghost_view) qc1_opt else qc2_opt
                }
        }
    }

    pure def timeout_qc_verify(qc: TimeoutQC): bool =  {
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
        }
    }

    // parts of the inductive invariant

    // Make sure that the timeout votes in the store are well-formed.
    val store_signed_timeout_all_inv: bool = {
        CORRECT.forall(id => and {
            val signed_votes = store_signed_timeout.get(id)
            signed_votes.subseteq(msgs_signed_timeout),
            /*
            signed_votes.forall(v1 => {
                signed_votes.forall(v2 => and {
                    // no duplicates
                    (v1.sig == v2.sig and v1.vote.view == v2.vote.view) implies (v1.vote == v2.vote),
                    // the quorums have an intersection
                    match (v1.high_commit_qc) {
                        | None => true
                        | Some(qc1) =>
                            match (v2.high_commit_qc) {
                                | None => true
                                | Some(qc2) => or {
                                    qc1.vote.view != qc2.vote.view,
                                    // TODO: use QUORUM_WEIGHT - F?
                                    qc1.agg_sig.intersect(qc2.agg_sig).size() >= SUBQUORUM_WEIGHT,
                                }
                            }
                    }
                })
            })
            */
        })
    }

    def msgs_signed_timeout_inv: bool = {
        msgs_signed_timeout.forall(v => {
            CORRECT.contains(v.sig) implies signed_timeout_vote_verify(v)
        })
    }
}