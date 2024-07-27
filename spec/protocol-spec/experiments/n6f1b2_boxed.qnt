// -*- mode: Bluespec; -*-
//
// A specification instance for n=6, f=1, and 2 Byzantine faults.
// We box the number of different steps.
module n6f1b2_boxed {
  import replica(
    CORRECT = Set("n0", "n1", "n2", "n3"),
    FAULTY = Set("n4", "n5"),
    WEIGHTS = Map("n0"->1, "n1"->1, "n2"->1, "n3"->1, "n4"->1, "n5"->1),
    REPLICA_KEYS = Map("n0"->"n0", "n1"->"n1", "n2"->"n2", "n3"->"n3", "n4"->"n4", "n5"->"n5"),
    N = 6,
    F = 1,
    VIEWS = 0.to(5),
    VALID_BLOCKS = Set("val_b0", "val_b1"),
    INVALID_BLOCKS = Set("inv_b3")
  ) as i from "../replica"

  // bounds on the number of steps
  pure val MAX_FAULTY_STEPS = 4
  pure val MAX_TIMEOUTS = 5
  pure val MAX_PROPOSER_STEPS = 2
  pure val MAX_ON_PROPOSAL_STEPS = 10
  pure val MAX_ON_COMMIT_STEPS = 10
  pure val MAX_ON_TIMEOUT_STEPS = 5
  pure val MAX_ON_NEW_VIEW_STEPS = 5

  var nfaulty_steps: int
  var ntimeouts: int
  var nproposer_steps: int
  var ncommit_steps: int
  var ntimeout_steps: int
  var nproposal_steps: int
  var nnew_view_steps: int

  action init: bool = all {
    i::init,
    nfaulty_steps' = 0,
    ntimeouts' = 0,
    nproposer_steps' = 0,
    ncommit_steps' = 0,
    ntimeout_steps' = 0,
    nproposal_steps' = 0,
    nnew_view_steps' = 0,
  }

  action step = {
    any {
      nondet id = oneOf(i::CORRECT)
      any {
        all {
          i::replica_view' = i::replica_view.set(id, i::replica_state.get(id).view),
          any {
              all {
                  ntimeouts < MAX_TIMEOUTS,
                  i::on_timer_is_finished(id),
                  ntimeouts' = ntimeouts + 1,
                  ncommit_steps' = ncommit_steps,
                  ntimeout_steps' = ntimeout_steps,
                  nproposal_steps' = nproposal_steps,
                  nnew_view_steps' = nnew_view_steps,
              },
              all {
                  ntimeout_steps < MAX_ON_TIMEOUT_STEPS,
                  i::msgs_signed_timeout != Set(),
                  nondet vote = oneOf(i::msgs_signed_timeout)
                  i::on_timeout(id, vote),
                  ntimeouts' = ntimeouts,
                  ncommit_steps' = ncommit_steps,
                  ntimeout_steps' = ntimeout_steps + 1,
                  nproposal_steps' = nproposal_steps,
                  nnew_view_steps' = nnew_view_steps,
              },
              all {
                  ncommit_steps < MAX_ON_COMMIT_STEPS,
                  i::msgs_signed_commit != Set(),
                  nondet signed_vote = oneOf(i::msgs_signed_commit)
                  i::on_commit(id, signed_vote),
                  ntimeouts' = ntimeouts,
                  ncommit_steps' = ncommit_steps + 1,
                  ntimeout_steps' = ntimeout_steps,
                  nproposal_steps' = nproposal_steps,
                  nnew_view_steps' = nnew_view_steps,
              },
              all {
                  nproposal_steps < MAX_ON_PROPOSAL_STEPS,
                  i::msgs_proposal != Set(),
                  nondet proposal = oneOf(i::msgs_proposal)
                  i::on_proposal(id, proposal),
                  ntimeouts' = ntimeouts,
                  ncommit_steps' = ncommit_steps,
                  ntimeout_steps' = ntimeout_steps,
                  nproposal_steps' = nproposal_steps + 1,
                  nnew_view_steps' = nnew_view_steps,
              },
              all {
                  nnew_view_steps < MAX_ON_NEW_VIEW_STEPS,
                  i::msgs_new_view != Set(),
                  nondet new_view = oneOf(i::msgs_new_view)
                  i::on_new_view(id, new_view),
                  ntimeouts' = ntimeouts,
                  ncommit_steps' = ncommit_steps,
                  ntimeout_steps' = ntimeout_steps,
                  nproposal_steps' = nproposal_steps,
                  nnew_view_steps' = nnew_view_steps + 1,
              }
          },
          i::leader' = i::leader,
          nfaulty_steps' = nfaulty_steps,
          nproposer_steps' = nproposer_steps,
        },
        all {
          nproposer_steps < MAX_PROPOSER_STEPS,
          // Non-deterministically choose the next block, use it only for the case of None below.
          nondet new_block = oneOf(i::VALID_BLOCKS)
          i::proposer_step(id, new_block),
          i::replica_view' = i::replica_view,
          i::leader' = i::leader,
          nfaulty_steps' = nfaulty_steps,
          ntimeouts' = ntimeouts,
          nproposer_steps' = nproposer_steps + 1,
          ncommit_steps' = ncommit_steps,
          ntimeout_steps' = ntimeout_steps,
          nproposal_steps' = nproposal_steps,
          nnew_view_steps' = nnew_view_steps,
        },
      },
      all {
        nfaulty_steps < MAX_FAULTY_STEPS,
        i::faulty_step,
        i::unchanged_replica,
        i::leader' = i::leader,
        nfaulty_steps' = nfaulty_steps + 1,
        ntimeouts' = ntimeouts,
        nproposer_steps' = nproposer_steps,
        ncommit_steps' = ncommit_steps,
        ntimeout_steps' = ntimeout_steps,
        nproposal_steps' = nproposal_steps,
        nnew_view_steps' = nnew_view_steps,
      }
    }
  }
}
