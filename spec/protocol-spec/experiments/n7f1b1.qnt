// A specification instance for n=6, f=1, and no Byzantine faults
module n7f1b1 {
  import replica(
    CORRECT = Set("n0", "n1", "n2", "n3", "n4", "n5"),
    FAULTY = Set("n6"),
    WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1, "n6"->1),
    REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5", "n6"->"n6"),
    N = 7,
    F = 1,
    VIEWS = 0.to(3),
    VALID_BLOCKS = Set("val_b0", "val_b1"),
    INVALID_BLOCKS = Set("inv_b3")
  ).* from "../replica"
}
