module n6f1b1_inductive {
  import inductive(
    I_CORRECT = Set("n0", "n1", "n2", "n3", "n4"),
    I_FAULTY = Set("n5"),
    I_N = 6,
    I_F = 1,
    I_VIEWS = 0.to(2),
    I_BLOCK_NUMBERS = Set(0),
    I_VALID_BLOCKS = Set("val_b0", "val_b1"),
    I_INVALID_BLOCKS = Set("inv_b3"),
    MAX_COMMIT_QC = 10,
    MAX_TIMEOUT_QC = 10
  ).* from "./inductive"
}