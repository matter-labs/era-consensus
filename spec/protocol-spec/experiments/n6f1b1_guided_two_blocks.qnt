// -*- mode: Bluespec; -*-
//
// A specification instance for n=6, f=1, and 1 Byzantine fault
//
// Due to import/export limitations of Quint, use the definitions starting with g_:
//
// quint verify --max-steps=50 --init=g_init --step=g_step --invariant=g_two_chained_blocks_example n6f1b1_two_blocks.qnt
module n6f1b1_two_blocks {
  import types.* from "../types"

  import guided_replica(
    G_CORRECT = Set("n0", "n1", "n2", "n3", "n4"),
    G_FAULTY = Set("n5"),
    G_WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
    G_REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5"),
    G_N = 6,
    G_F = 1,
    G_VIEWS = 0.to(5),
    G_VALID_BLOCKS = Set("val_b0", "val_b1"),
    G_INVALID_BLOCKS = Set("inv_b3"),
    G_INPUT_TRACE = [
      ProposerStep("n0"),
      OnProposalStep("n0"), OnProposalStep("n1"), OnProposalStep("n2"), OnProposalStep("n3"), OnProposalStep("n4"),
      OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"),
      OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"), OnCommitStep("n1"),
      OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"), OnCommitStep("n2"),
      OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"),
      OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"), OnCommitStep("n4"),
      ProposerStep("n0"),
      OnProposalStep("n0"), OnProposalStep("n1"), OnProposalStep("n2"), OnProposalStep("n3"), OnProposalStep("n4"),
      OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"),
    ]
  ).* from "../guided_replica"
}
