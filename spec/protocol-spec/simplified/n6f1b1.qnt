// -*- mode: Bluespec; -*-
// A specification instance for n=6, f=1, and 1 Byzantine fault
module n6f1b1 {
  import replica(
    CORRECT = Set("n0", "n1", "n2", "n3", "n4"),
    FAULTY = Set("n5"),
    N = 6,
    F = 1,
    VIEWS = 0.to(5),
    VALID_BLOCKS = Set("val_b0", "val_b1"),
    INVALID_BLOCKS = Set("inv_b3")
  ).* from "./replica"
}
