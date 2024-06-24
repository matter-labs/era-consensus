// A specification instance for n=6, f=1, and 1 Byzantine fault
//
/// Test a liveness issue where some validators have the HighQC but don't have the block payload and have to wait for it,
/// while some other validators have the payload but don't have the HighQC and cannot finalize the block, and therefore
/// don't gossip it, which causes a deadlock unless the one with the HighQC moves on and broadcasts what they have, which
/// should cause the others to finalize the block and gossip the payload to them in turn.
//
// Due to import/export limitations of Quint, use the definitions starting with g_:
//
// quint verify --max-steps=50 --init=g_init --step=g_step --invariant=g_all_invariants n6f1b1_dead_lock.qnt
module n6f1b1_two_blocks {
    import types.* from "../types"

    import guided_replica(
        G_CORRECT = Set("n0", "n1", "n2", "n3", "n4", "n5"),
        // no random byzantine behaviour
        G_FAULTY = Set(),
        G_WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
        G_REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5"),
        G_N = 6,
        G_F = 1,
        G_VIEWS = 0.to(5),
        G_VALID_BLOCKS = Set("val_b0", "val_b1"),
        G_INVALID_BLOCKS = Set("inv_b3"),
        G_INPUT_TRACE = [
            // n0 proposes a block number 0
            ProposerStep("n0"),
            OnProposalStep("n0"), OnProposalStep("n1"), OnProposalStep("n2"), OnProposalStep("n3"), OnProposalStep("n4"), OnProposalStep("n5"),
            // node 0 is slow, node 1 finalizes the block number 0
            OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), 
            // n1 is in view 2

            // all replicas but n0 timeout
            OnTimerIsFinishedStep("n2"), OnTimerIsFinishedStep("n3"), OnTimerIsFinishedStep("n4"), OnTimerIsFinishedStep("n5"),
            // n2-n5 catch up on CommitQC. They should add the block number 0 to their finalized blocks.
            OnNewViewStep({ id: "n2", view: 2 }), OnNewViewStep({ id: "n3", view: 2 }),
            OnNewViewStep({ id: "n4", view: 2 }), OnNewViewStep({ id: "n5", view: 2 }),
            // n1 proposes a block number 1
            ProposerStep("n1"),
            OnProposalStep("n1"), OnProposalStep("n2"), OnProposalStep("n3"), OnProposalStep("n4"), OnProposalStep("n5"),
        ]
    ).* from "../guided_replica"
}