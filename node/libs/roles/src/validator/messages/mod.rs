//! Messages exchanged between validators.

mod block;
mod committee;
mod consensus;
mod discovery;
mod genesis;
mod leader_proposal;
mod msg;
mod replica_commit;
mod replica_new_view;
mod replica_timeout;
#[cfg(test)]
mod tests;

pub use block::*;
pub use committee::*;
pub use consensus::*;
pub use discovery::*;
pub use genesis::*;
pub use leader_proposal::*;
pub use msg::*;
pub use replica_commit::*;
pub use replica_new_view::*;
pub use replica_timeout::*;

/// Calculate the maximum allowed weight for faulty replicas, for a given total weight.
pub fn max_faulty_weight(total_weight: u64) -> u64 {
    // Calculate the allowed maximum weight of faulty replicas. We want the following relationship to hold:
    //      n = 5*f + 1
    // for n total weight and f faulty weight. This results in the following formula for the maximum
    // weight of faulty replicas:
    //      f = floor((n - 1) / 5)
    (total_weight - 1) / 5
}

/// Calculate the consensus quorum threshold, the minimum votes' weight necessary to finalize a block,
/// for a given committee total weight.
pub fn quorum_threshold(total_weight: u64) -> u64 {
    total_weight - max_faulty_weight(total_weight)
}

/// Calculate the consensus subquorum threshold, the minimum votes' weight necessary to trigger a reproposal,
/// for a given committee total weight.
pub fn subquorum_threshold(total_weight: u64) -> u64 {
    total_weight - 3 * max_faulty_weight(total_weight)
}
