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
//!
//!

use std::cmp::min;

/// A group of nodes that can talk to each other.
type Partition<'a, T> = Vec<&'a T>;
/// A division of nodes into disjunct partitions, with no communication between different groups.
type Partitioning<'a, T> = Vec<Partition<'a, T>>;

/// Generate all possible partitioning of `items` into `num_partitions` groups.
///
/// The idea is to fill out a table such as this:
/// ```text
///   P1 P2 P3  
/// A  *
/// B  *
/// C     *
/// D       *
/// ```
/// where the rows are the items and the columns the partitions.
///
/// One goal is to minimise the redundant combinations. For example if we want to
/// partition `[A, B, C]` into two groups, we want `[{A, B}, {C}]` to appear in the
/// results, but not `[{C}, {A, B}]`, or `[{B, A}, {C}]` as they are the same, but
/// we do want `[{A, C}, {B}]` because they have different labels.
pub fn partitions<'a, T>(items: &[&'a T], num_partitions: usize) -> Vec<Partitioning<'a, T>> {
    Partitioner::generate(items.into(), num_partitions)
}

/// Recursive partition generator.
struct Partitioner<'a, T> {
    items: Partition<'a, T>,
    num_items: usize,
    num_partitions: usize,
    // All collected complete partitionings
    output: Vec<Partitioning<'a, T>>,
    // Partially complete partitioning currently being built
    acc: Partitioning<'a, T>,
}

impl<'a, T> Partitioner<'a, T> {
    /// Generate all possible partitioning.
    fn generate(items: Partition<'a, T>, num_partitions: usize) -> Vec<Partitioning<'a, T>> {
        if items.is_empty() || num_partitions == 0 {
            // Impossible to partition.
            Vec::new()
        } else if num_partitions == items.len() {
            // Each items stands alone.
            vec![items.into_iter().map(|i| vec![i]).collect()]
        } else if num_partitions == 1 {
            // All items are in a single partition.
            vec![vec![items]]
        } else {
            // Create empty partitions for each slot.
            let acc = (0..num_partitions)
                .into_iter()
                .map(|_| Vec::new())
                .collect();

            let mut ps = Self {
                num_items: items.len(),
                num_partitions,
                items,
                output: Vec::new(),
                acc,
            };
            // Generate all combinations
            ps.go(0, 0);
            // Take the results
            ps.output
        }
    }

    /// Recursively generate partitions.
    ///
    /// The algorithm goes item-by-item and has two indexes depending on what has happened
    /// above it: the minimum and the maximum partition that it can (or has to) insert into.
    ///
    /// Take the table in [partitions] as an example with 4 items and 3 partitions.
    /// * `A` is the first item, so it can only go into `P1` - `P2` can't be used
    /// while `P1` is empty, otherwise we'd be generating redundant combinations. After that,
    /// * `B` can go into minimally into `P1` because there are 2 more items after it, which
    /// is enough to fill all remaining partitions; or it can go into `P2`, because `P1` is filled.
    /// * `C` depends on what we did with `B`: if `B` is in `P1` then `C` has to minimally go into
    /// `P2` to make sure no paritition will be left empty at the end; if `B` is in `P2` then `C`
    /// can go either in `P1`, `P2` or `P3`
    ///
    /// The algorithm has to traverse all possible placements with backtracking.
    fn go(&mut self, idx: usize, first_empty: usize) {
        // If we're beyond the last item, emit the complete combination.
        if idx == self.num_items {
            self.output.push(self.acc.clone());
            return;
        }
        // Number of remaining items, including this one.
        let rem_items = self.num_items - idx;
        // Number of remaining empty partitions.
        let rem_empty = self.num_partitions.saturating_sub(first_empty);
        // Index of the last partition.
        let last_part = self.num_partitions - 1;
        // The maximum partition we can put into is the first empty one.
        let max_part = min(first_empty, last_part);
        // The minimum partition we have to put the item into is the one that still allows the remaining empty ones to be filled.
        let min_part = if rem_empty == rem_items { max_part } else { 0 };
        // Now generate all possible allocations.
        for ins_part in min_part..=max_part {
            self.acc[ins_part].push(self.items[idx]);
            self.go(
                idx + 1,
                if ins_part == first_empty {
                    first_empty + 1
                } else {
                    first_empty
                },
            );
            // Backtrack
            self.acc[ins_part].pop();
        }
    }
}

#[cfg(test)]
mod tests;
