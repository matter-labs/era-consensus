# Experimental Results

This page summarizes model checking experiments.

## 1. N=6, F = 1, B = 1

This is the configuration [n6f1b1][], which comprises 5 correct replicas and one Byzantine replica.

We have run randomized symbolic executions with Apalache with the budget of
(feasible) 100 symbolic runs, up to 25 steps in each run. The model checker
checked the following invariants:

 - `agreement_inv`
 - `committed_blocks_have_justification_inv`
 - `no_proposal_equivocation_inv`
 - `no_commit_equivocation_inv`
 - `no_timeout_equivocation_inv`
 - `no_new_view_equivocation_inv`
 - `store_signed_commit_all_inv`
 - `store_signed_timeout_all_inv`
 - `view_justification_inv`
 - `justification_is_supported_inv`
 - `one_high_vote_in_timeout_qc_inv`
 - `one_commit_quorum_inv`
 - `all_replicas_high_commit_qc_inv`
 - `all_replicas_high_timeout_qc_inv`
 - `all_replicas_high_vote_inv`
 - `msgs_signed_timeout_inv`
 - `msgs_signed_commit_inv`
 - `timeout_high_vote_is_highest_inv`

After we fixed a technical issue in `agreement_inv`, no further invariant
violations were found.

[n6f1b1]: ./n6f1b1.qnt