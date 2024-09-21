module n6f1b1_inductive {
  import inductive(
    ICORRECT = Set("n0", "n1", "n2", "n3", "n4"),
    IFAULTY = Set("n5"),
    IN = 6,
    IF = 1,
    IVIEWS = 0.to(2),
    IBLOCK_NUMBERS = Set(0),
    IVALID_BLOCKS = Set("val_b0", "val_b1"),
    IINVALID_BLOCKS = Set("inv_b3"),
    MAX_COMMIT_QC = 2,
    MAX_TIMEOUT_QC = 2
  ).* from "./inductive"
}