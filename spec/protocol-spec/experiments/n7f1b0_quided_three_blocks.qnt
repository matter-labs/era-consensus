// A specification instance for n=7, f=1, and 0 byzantine nodes.
//
// Due to import/export limitations of Quint, use the definitions starting with g_:
//
// JVM_ARGS=-Xmx16G quint verify --max-steps=160 --init=g_init --step=g_step --invariant=g_all_invariants n7f1b0_three_blocks.qnt
module n6f1b1_two_blocks {
    import types.* from "../types"

    import guided_replica(
        G_CORRECT = Set("n0", "n1", "n2", "n3", "n4", "n5", "n6"),
        // no random byzantine behaviour
        G_FAULTY = Set(),
        G_WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1, "n6"->1),
        G_REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5", "n6"->"n6"),
        G_N = 7,
        G_F = 1,
        G_VIEWS = 0.to(5),
        G_VALID_BLOCKS = Set("val_b0", "val_b1"),
        G_INVALID_BLOCKS = Set("inv_b3"),
        G_INPUT_TRACE = [

            ProposerStep("n1"),

            OnProposalStep("n0"),
            OnProposalStep("n1"),
            OnProposalStep("n2"),
            OnProposalStep("n3"),
            OnProposalStep("n4"),
            OnProposalStep("n5"),

            OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), 
            OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), 
            OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"),
            OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"),
            OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"),
            OnCommitStep("n5"), OnCommitStep("n5"), OnCommitStep("n5"), OnCommitStep("n5"), OnCommitStep("n5"), OnCommitStep("n5"),

            ProposerStep("n4"),
            
            OnProposalStep("n0"),
            OnProposalStep("n1"),
            OnProposalStep("n2"),
            OnProposalStep("n3"),
            OnProposalStep("n4"),
            OnProposalStep("n5"),

            OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), 
            OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), 
            OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"),
            OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"),
            OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"),
            OnCommitStep("n5"), OnCommitStep("n5"), OnCommitStep("n5"), OnCommitStep("n5"), OnCommitStep("n5"), OnCommitStep("n5"),

            ProposerStep("n5"),
            
            OnProposalStep("n0"),
            OnProposalStep("n1"),
            OnProposalStep("n2"),
            OnProposalStep("n3"),
            OnProposalStep("n4"),
            OnProposalStep("n5"),

            OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), 
        ]
    ).* from "../guided_replica"
}