use std::collections::BTreeSet;

use super::Twin;

/// A cluster holds all the nodes in the simulation, some of which are twins.
pub struct Cluster<T> {
    /// All nodes, twins and originals.
    nodes: Vec<T>,
    /// The ideal size of the committee, ie. the `n` in the `n = 5 * f + 1`.
    num_replicas: usize,
}

impl<T> Cluster<T>
where
    T: Twin,
    T::Key: Ord,
{
    /// Take a number of non-twin replicas and produce a cluster with a desired number of twins.
    pub fn new(replicas: Vec<T>, num_twins: usize) -> Self {
        // Check that the replicas aren't twins.
        let keys = BTreeSet::from_iter(replicas.iter().map(|r| r.key()));
        assert_eq!(keys.len(), replicas.len(), "replicas have unique keys");

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

    /// The number of signatures required for a quorum, ie. `4 * f + 1`
    pub fn quorum_size(&self) -> usize {
        self.num_faulty() * 4 + 1
    }

    /// The number of signatures required for a high-vote, ie. `2 * f + 1`
    pub fn subqourum_size(&self) -> usize {
        self.num_faulty() * 2 + 1
    }

    /// All nodes in the cluster, replicas and twins.
    pub fn nodes(&self) -> &[T] {
        &self.nodes
    }
}
