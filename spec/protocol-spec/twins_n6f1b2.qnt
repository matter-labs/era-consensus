// -*- mode: Bluespec; -*-
// This is an explorative module applying the Twins approach 
// to model byzantine behaviour with 2 faulty replicas and 4 correct replicas.
module twins {
  // A specification instance for n=6, f=1 (threshold), 4 correct replicas, and 2 faulty replicas equivocating
  import replica(
    CORRECT = Set("n0", "n1", "n2", "n3", "n4_1", "n4_2", "n5_1", "n5_2"),
    REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4_1" -> "n4", "n4_2" -> "n4", "n5_1"->"n5", "n5_2"->"n5"),
    // Faulty set is empty to disable randomized byzantine behaviour generator and use correct replicas twins instead
    FAULTY = Set(),
    WEIGHTS = Map("n0" -> 1, "n1" -> 1, "n2" -> 1, "n3" -> 1, "n4_1" -> 1, "n4_2" -> 0, "n5_1" -> 1, "n5_2" -> 0),
    N = 6,
    F = 1,
    VIEWS = 0.to(5),
    VALID_BLOCKS = Set("val_b0", "val_b1"),
    INVALID_BLOCKS = Set("inv_b3")
  ) as i from "./replica"

  action init = all {
    // all leaders are byzantine:
    // non-deterministically choose the leader function to choose between the twins replicas
    nondet ldr = i::VIEWS.setOfMaps(Set("n4", "n5")).oneOf()
    i::init_view_1_with_leader(ldr),
    

    // prevent accidental misconfigurations of this module
    assert(i::N == i::TOTAL_WEIGHT),
    // 2 replicas from the CORRECT set have twins that makes them byzantine
    assert(i::N + 2 == i::CORRECT.size()),
    assert(i::FAULTY.size()==0)
  }


  action step = i::correct_step
}
