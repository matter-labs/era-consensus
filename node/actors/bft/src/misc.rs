//! Miscellaneous functions related to the consensus.

/// Calculate the consensus threshold, the minimum number of votes for any consensus action to be valid,
/// for a given number of replicas.
pub fn consensus_threshold(num_validators: usize) -> usize {
    let faulty_replicas = faulty_replicas(num_validators);

    // Return the consensus threshold, which is simply:
    //      t = n - f
    num_validators - faulty_replicas
}

/// Calculate the maximum number of faulty replicas, for a given number of replicas.
pub fn faulty_replicas(num_validators: usize) -> usize {
    // Calculate the allowed maximum number of faulty replicas. We want the following relationship to hold:
    //      n = 5*f + 1
    // for n total replicas and f faulty replicas. This results in the following formula for the maximum
    // number of faulty replicas:
    //      f = floor((n - 1) / 5)
    // Because of this, it doesn't make sense to have 5*f + 2 or 5*f + 3 replicas. It won't increase the number
    // of allowed faulty replicas.
    (num_validators - 1) / 5
}
