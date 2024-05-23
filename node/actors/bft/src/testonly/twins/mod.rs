//! Utilities to implement some of the ideas in the [Twins paper](https://arxiv.org/pdf/2004.10617) for testing Byzantine behaviour.
//!
//! The main concepts are:
//! * If we have a committee of n=5f+1 nodes, then up to f nodes can have a _twin_ in the network with the same validator key, but different network keys.
//!   These nodes behave in an honest way in terms of state machine implementation, but because their memory is isolated and they receive different messages,
//!   they can exhibit Byzantine behaviour such as equivocation and amnesia.
//! * In each round (view) we partition the nodes into disjunct groups, and the mock network in the test will only allow communication between nodes in the
//!   same partition in that round. This way the test controls the sub-quorum sizes, e.g. by reshuffling the partitions it can show 2f+1 high votes on the
//!   previous leader's proposal.
//! * The test controls who is the leader in each round; when a validator key is leading, its twin is also leading.
//! * The test generates all possible combinations and permutations, and prune to keep the interesting ones (e.g. make sure there is 4f+1 sized partitions eventually).
//!   In practice we only take a sample because with practical committee sizes there is a combinatorial explosion.

mod partition;
mod scenario;

pub use partition::*;
pub use scenario::*;

/// Abstraction for nodes which can have a twin.
pub trait Twin
where
    Self: PartialEq,
{
    /// Validator public key.
    type Key: PartialEq;

    /// The validator key of this node, used to identify who is leading a round.
    ///
    /// The twin and its original both have the same key.
    fn key(&self) -> &Self::Key;

    /// Create a twin for the node.
    fn to_twin(&self) -> Self;

    /// Check if the other node is a twin of this one.
    ///
    /// This is a reflexive, they are twins of each other.
    fn is_twin_of(&self, other: &Self) -> bool {
        self != other && self.key() == other.key()
    }
}

#[cfg(test)]
mod tests;
