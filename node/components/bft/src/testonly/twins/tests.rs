use super::{splits, Cluster, HasKey, ScenarioGenerator, Split, Twin};
use crate::testonly::twins::unique_key_count;
use rand::Rng;
use std::{
    collections::{BTreeSet, HashSet},
    fmt::Debug,
};
use zksync_concurrency::ctx;

#[test]
fn test_splits() {
    let got = splits(&["foo", "bar", "baz"], 2);
    let want: HashSet<Split<_>> = [
        vec![vec![&"foo"], vec![&"bar", &"baz"]],
        vec![vec![&"foo", &"bar"], vec![&"baz"]],
        vec![vec![&"foo", &"baz"], vec![&"bar"]],
    ]
    .into();
    assert_eq!(got.len(), want.len());
    let got = HashSet::from_iter(got);
    assert_eq!(got, want);
}

#[test]
fn prop_splits() {
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();
    for _ in 0..100 {
        let num_partitions = rng.gen_range(0..=3);
        let num_items = rng.gen_range(0..=7);

        let mut items = Vec::new();
        for i in 0..=num_items {
            items.push(i);
        }

        let got = splits(&items, num_partitions);
        let got_len = got.len();
        let got = BTreeSet::from_iter(got.into_iter().map(|ps| {
            BTreeSet::from_iter(
                ps.into_iter()
                    .map(|p| BTreeSet::from_iter(p.into_iter().copied())),
            )
        }));

        let want = splits_naive(items, num_partitions);

        assert_eq!(
            got_len,
            want.len(),
            "length num_items={num_items} num_partitions={num_partitions}"
        );

        assert_eq!(
            got, want,
            "values num_items={num_items} num_partitions={num_partitions}"
        );
    }
}

#[test]
fn prop_twin() {
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();
    let node1 = TestNode {
        id: format!("{}", rng.gen::<char>()),
        key: rng.gen(),
    };
    let node2 = TestNode {
        id: format!("not-{}", node1.id),
        key: node1.key + 1,
    };
    let twin = node1.to_twin();
    assert_eq!(node1.key, twin.key, "key doesn't change");
    assert!(node1.id != twin.id, "id changes");
    assert!(node1.is_twin_of(&twin), "node1 -> twin");
    assert!(twin.is_twin_of(&node1), "twin -> node1");
    assert!(!twin.is_twin_of(&node2), "twin -> node2");
    assert_eq!(twin.to_twin(), node1);
}

#[test]
fn test_scenario_generator() {
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();

    let replicas = ["A", "B", "C", "D", "E", "F"]
        .into_iter()
        .enumerate()
        .map(|(i, id)| TestNode {
            key: i as u64,
            id: id.to_string(),
        })
        .collect::<Vec<_>>();

    let num_rounds = 5;
    let max_partitions = 3;
    let num_replicas = replicas.len();
    let num_twins = 1;
    let quorum_size = 4 * num_twins + 1;

    let cluster = Cluster::new(replicas, num_twins);
    let generator = ScenarioGenerator::<_, 2>::new(&cluster, num_rounds, max_partitions);

    for _ in 0..100 {
        let scenario = generator.generate_one(rng);

        assert_eq!(scenario.rounds.len(), num_rounds);

        for rc in &scenario.rounds {
            assert!(
                cluster.replicas().iter().any(|r| r.key == *rc.leader),
                "leader exists"
            );
            for pp in &rc.phase_partitions {
                assert!(
                    pp.len() <= max_partitions,
                    "no more partitions than configured"
                );
                assert_eq!(
                    pp.iter().map(|p| p.len()).sum::<usize>(),
                    num_replicas + num_twins,
                    "all nodes partitioned"
                );
            }
        }

        assert!(
            scenario.rounds.iter().any(|rc| rc
                .phase_partitions
                .iter()
                .any(|ps| ps.iter().any(|p| unique_key_count(p) >= quorum_size))),
            "eventually has quorum"
        );
    }
}

#[test]
fn prop_cluster() {
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();

    for _ in 0..50 {
        let num_replicas = rng.gen_range(1..=15);

        let replicas = (0..num_replicas)
            .map(|i| TestNode {
                key: i as u64,
                id: i.to_string(),
            })
            .collect::<Vec<_>>();

        let num_twins = rng.gen_range(0..=num_replicas);
        let cluster = Cluster::new(replicas, num_twins);

        assert_eq!(cluster.replicas().len(), num_replicas);
        assert_eq!(cluster.twins().len(), num_twins);
        assert_eq!(cluster.quorum_size(), cluster.num_replicas() * 4 / 5 + 1);
    }
}

#[derive(Debug, Clone, PartialEq)]
struct TestNode {
    id: String,
    key: u64,
}

impl HasKey for TestNode {
    type Key = u64;
    fn key(&self) -> &Self::Key {
        &self.key
    }
}

impl Twin for TestNode {
    fn to_twin(&self) -> Self {
        let id = match self.id.strip_suffix('\'') {
            Some(id) => id.to_string(),
            None => format!("{}'", self.id),
        };
        Self { id, key: self.key }
    }
}

/// Naive implementation of the partitioning to test against.
fn splits_naive<T: Ord + Eq + Clone + Debug>(
    items: Vec<T>,
    num_partitions: usize,
) -> BTreeSet<BTreeSet<BTreeSet<T>>> {
    // Seed the accumulator with the single empty partition
    let mut acc = BTreeSet::new();
    acc.insert(vec![BTreeSet::new(); num_partitions]);

    // Allocate each item into every possible partition, replacing the accumulator each time.
    for item in &items {
        acc = acc
            .into_iter()
            .flat_map(|ps| {
                (0..ps.len()).map(move |i| {
                    let mut ps = ps.clone();
                    ps[i].insert(item.clone());
                    ps
                })
            })
            .collect();
    }

    acc.into_iter()
        // Get rid of cases where some partitions were left empty.
        .filter(|ps| ps.iter().all(|p| !p.is_empty()))
        // Make it unique so every combination only appears once.
        .map(BTreeSet::from_iter)
        .collect()
}
