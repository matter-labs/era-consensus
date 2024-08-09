// -*- mode: Bluespec; -*-
// A state monitor that allows us to compare new state against the old states.
// For example, we compare phase transitions.
// To do so, we save the old replica states in old_replica_state.
module state_monitor {
    import types.* from "./types"
    import option.* from "./option"

    import replica(
      CORRECT = Set("n0", "n1", "n2", "n3", "n4"),
      FAULTY = Set("n5"),
      WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
      N = 5,
      F = 1,
      VIEWS = 0.to(20),
      VALID_BLOCKS = Set("val_b0", "val_b1", "val_b2"),
      INVALID_BLOCKS = Set("inv_b3")
    ) as i from "./replica"

    var old_replica_state: Option[ReplicaId -> ReplicaState]

    action init = all {
      i::init,
      old_replica_state' = None,
    }

    action step = all {
      old_replica_state' = Some(i::replica_state),
      i::step,
    }

    // Check the phase transitions as a state invariant:
    // Only two possible phase paths:
    // Within the same view, a replica’s phase can only change through two paths.
    // 1) Prepare→Commit→Timeout or 2) Prepare→Timeout.
    val phase_inv = {
      def for_replica(id: ReplicaId): bool = {
        match (old_replica_state) {
          | None =>
            true
          | Some(old) =>
            val same_view = (old.get(id).view == i::replica_state.get(id).view)
            val old_phase = old.get(id).phase
            val new_phase = i::replica_state.get(id).phase
            if (same_view) or {
              old_phase == new_phase,
              old_phase == PhasePrepare and new_phase == PhaseCommit,
              old_phase == PhaseCommit and new_phase == PhaseTimeout,
              old_phase == PhasePrepare and new_phase == PhaseTimeout,
            } else or {
              new_phase == PhasePrepare,
              old_phase == PhaseCommit or old_phase == PhaseTimeout
            }
        }
      }

      i::CORRECT.forall(id => for_replica(id))
    }
}
