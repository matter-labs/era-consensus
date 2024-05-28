use std::collections::BTreeSet;

use rand::{seq::SliceRandom, Rng};

use super::{splits, HasKey, Split, Twin};

/// A cluster holds all the nodes in the simulation, some of which are twins.
pub struct Cluster<T> {
    /// All nodes, twins and originals.
    nodes: Vec<T>,
    /// The ideal size of the committee, ie. the `n` in the `n = 5 * f + 1` equation from the spec.
    /// Note that the tests don't assume that we have exactly `f=(n-1)/5` twins.
    num_replicas: usize,
}

impl<T> Cluster<T>
where
    T: Twin,
{
    /// Take a number of non-twin replicas and produce a cluster with a desired number of twins.
    pub fn new(replicas: Vec<T>, num_twins: usize) -> Self {
        // Check that the replicas aren't twins.
        assert_eq!(
            unique_key_count(&replicas),
            replicas.len(),
            "replicas must have unique keys"
        );

        let twins = replicas
            .iter()
            .take(num_twins)
            .map(|r| r.to_twin())
            .collect::<Vec<_>>();

        let num_replicas = replicas.len();

        let mut nodes = replicas;
        nodes.extend(twins);

        Self {
            nodes,
            num_replicas,
        }
    }

    /// The number of nodes in the cluster, both replicas and twins.
    pub fn num_nodes(&self) -> usize {
        self.nodes.len()
    }

    /// The number of original replicas, ie. not twins.
    ///
    /// This is the `n` in the `n = 5 * f + 1` equation.
    pub fn num_replicas(&self) -> usize {
        self.num_replicas
    }

    /// The number of twins in the cluster.
    pub fn num_twins(&self) -> usize {
        self.num_nodes() - self.num_replicas()
    }

    /// The number of faulty nodes the committee can tolerate for the `n = 5 * f + 1` relation to hold.
    pub fn num_faulty(&self) -> usize {
        (self.num_replicas - 1) / 5
    }

    /// The number of signatures required for a quorum, ie. `4 * f + 1` or more generally `n - f`, which in this case equals `n*4/5+1`
    pub fn quorum_size(&self) -> usize {
        self.num_replicas() - self.num_faulty()
    }

    /// The number of signatures required for a high-vote, ie. `2 * f + 1`
    pub fn subquorum_size(&self) -> usize {
        self.num_faulty() * 2 + 1
    }

    /// All nodes in the cluster, replicas and twins.
    pub fn nodes(&self) -> &[T] {
        &self.nodes
    }

    /// The original replicas in the cluster.
    pub fn replicas(&self) -> &[T] {
        &self.nodes[..self.num_replicas]
    }

    /// The twins we created in the cluster.
    pub fn twins(&self) -> &[T] {
        &self.nodes[self.num_replicas..]
    }
}

/// The configuration for a single round.
pub struct RoundConfig<'a, T: HasKey> {
    pub leader: &'a T::Key,
    pub partitions: Split<'a, T>,
}

/// Configuration for a number of rounds.
pub struct Scenario<'a, T: HasKey> {
    pub rounds: Vec<RoundConfig<'a, T>>,
}

pub struct ScenarioGenerator<'a, T>
where
    T: Twin,
{
    /// Number of rounds (views) to simulate in a scenario
    num_rounds: usize,
    /// Unique leader keys.
    keys: Vec<&'a T::Key>,
    /// All splits of various sizes we can choose in a round.
    splits: Vec<Split<'a, T>>,
}

impl<'a, T> ScenarioGenerator<'a, T>
where
    T: Twin,
{
    /// Initialise a scenario generator from a cluster.
    pub fn new(cluster: &'a Cluster<T>, num_rounds: usize, max_partitions: usize) -> Self {
        assert!(!cluster.nodes().is_empty(), "empty cluster");

        // Potential leaders
        let keys = cluster.replicas().iter().map(|r| r.key()).collect();

        // Create all possible partitionings; the paper considers 2 or 3 partitions to be enough.
        let splits = (1..=max_partitions).flat_map(|np| splits(cluster.nodes(), np));

        // Prune partitionings so that all of them have at least one where a quorum is possible.
        // Alternatively we could keep all and make sure every scenario has eventually one with a quorum, for liveness.
        let splits = splits
            .filter(|ps| {
                ps.iter()
                    .any(|p| unique_key_count(p) >= cluster.quorum_size())
            })
            .collect();

        Self {
            num_rounds,
            keys,
            splits,
        }
    }

    /// Generate a single run for the agreed upon number of rounds.
    pub fn generate_one(&self, rng: &mut impl Rng) -> Scenario<'a, T> {
        let mut rounds = Vec::new();
        // We could implement this with or without replacement.
        // In practice there probably are so many combinations that it won't matter.
        // For example to tolerate 2 faulty nodes we need 11 replicas, with 2 twins that's 13,
        // which results in hundreds of thousands of potential partitionings, multiplied
        // by 11 different possible leaders in each round.
        for _ in 0..self.num_rounds {
            rounds.push(RoundConfig {
                leader: self.keys.choose(rng).cloned().unwrap(),
                partitions: self.splits.choose(rng).cloned().unwrap(),
            })
        }

        Scenario { rounds }
    }
}

/// Count the number of node keys that do not have twins in the group.
pub fn unique_key_count<T>(nodes: &[T]) -> usize
where
    T: HasKey,
{
    nodes.iter().map(|n| n.key()).collect::<BTreeSet<_>>().len()
}
