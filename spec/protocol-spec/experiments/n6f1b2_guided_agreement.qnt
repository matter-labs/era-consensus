// -*- mode: Bluespec; -*-
//
// A specification instance for n=6, f=1, and 2 Byzantine faults
//
// Due to import/export limitations of Quint, use the definitions starting with g_:
//
// quint verify --max-steps=50 --init=g_init --step=g_step --invariant=g_agreement_inv n6f1b2_agreement.qnt
module n6f1b2_agreement {
  import types.* from "../types"

  import guided_replica(
    G_CORRECT = Set("n0", "n1", "n2", "n3"),
    G_FAULTY = Set("n4", "n5"),
    G_WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
    G_REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5"),
    G_N = 6,
    G_F = 1,
    G_VIEWS = 0.to(5),
    G_VALID_BLOCKS = Set("val_b0", "val_b1"),
    G_INVALID_BLOCKS = Set("inv_b3"),
    G_INPUT_TRACE = [
      FaultyStep,
      OnProposalStep("n0"), OnProposalStep("n1"), OnProposalStep("n2"),
      FaultyStep,
      OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"), OnCommitStep("n0"),
      FaultyStep,
      OnProposalStep("n3"),
      OnTimerIsFinishedStep("n1"), OnTimerIsFinishedStep("n2"), OnTimerIsFinishedStep("n3"),
      FaultyStep, FaultyStep, FaultyStep,
      OnNewViewStep({id:"n1", view:2}), OnNewViewStep({id:"n2", view:2}), OnNewViewStep({id:"n3", view:2}),
      FaultyStep,
      OnProposalStep("n1"), OnProposalStep("n2"), OnProposalStep("n3"),
      FaultyStep,
      OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"), OnCommitStep("n3"),
    ]
  ).* from "../guided_replica"
}
