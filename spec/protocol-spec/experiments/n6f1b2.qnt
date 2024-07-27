// -*- mode: Bluespec; -*-
//
// A specification instance for n=6, f=1, and 2 Byzantine faults
module n6f1b2 {
  import replica(
    CORRECT = Set("n0", "n1", "n2", "n3"),
    FAULTY = Set("n4", "n5"),
    WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
    REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5"),
    N = 6,
    F = 1,
    VIEWS = 0.to(3),
    VALID_BLOCKS = Set("val_b0", "val_b1"),
    INVALID_BLOCKS = Set("inv_b3")
  ).* from "../replica"
}
