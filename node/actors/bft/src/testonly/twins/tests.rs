use std::{
    collections::{BTreeSet, HashSet},
    fmt::Debug,
};

use rand::Rng;
use zksync_concurrency::ctx;

use super::{partitions, Partitioning};

#[test]
fn test_partitions() {
    let got = partitions(&["foo", "bar", "baz"], 2);
    let want: HashSet<Partitioning<_>> = HashSet::from_iter(vec![
        vec![vec![&"foo"], vec![&"bar", &"baz"]],
        vec![vec![&"foo", &"bar"], vec![&"baz"]],
        vec![vec![&"foo", &"baz"], vec![&"bar"]],
    ]);
    assert_eq!(got.len(), want.len());
    let got = HashSet::from_iter(got);
    assert_eq!(got, want);
}

#[test]
fn prop_partitions() {
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();
    for _ in 0..100 {
        let num_partitions = rng.gen_range(0..=3);
        let num_items = rng.gen_range(0..=7);

        let mut items = Vec::new();
        for i in 0..=num_items {
            items.push(i);
        }

        let got = partitions(&items, num_partitions);
        let got_len = got.len();
        let got = BTreeSet::from_iter(got.into_iter().map(|ps| {
            BTreeSet::from_iter(
                ps.into_iter()
                    .map(|p| BTreeSet::from_iter(p.into_iter().copied())),
            )
        }));

        let want = partitions_naive(items, num_partitions);

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

/// Naive implementation of the partitioning to test against.
fn partitions_naive<T: Ord + Eq + Clone + Debug>(
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
