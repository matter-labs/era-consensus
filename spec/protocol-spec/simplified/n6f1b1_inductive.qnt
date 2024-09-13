module n6f1b1_inductive {
  import inductive(
    ICORRECT = Set("n0", "n1", "n2", "n3", "n4", "n5"),
    IFAULTY = Set(),
    IWEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
    IREPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5"),
    IN = 6,
    IF = 1,
    IVIEWS = 0.to(3),
    IBLOCK_NUMBERS = Set(0, 1),
    IVALID_BLOCKS = Set("val_b0", "val_b1"),
    IINVALID_BLOCKS = Set("inv_b3"),
    MAX_COMMIT_QC = 4,
    MAX_TIMEOUT_QC = 4
  ).* from "./inductive"
}