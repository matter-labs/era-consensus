use std::collections::HashSet;

use super::{partitions, Partitioning};

#[test]
fn test_partitions() {
    let got = partitions(&[&1, &2, &3], 2);
    let want: HashSet<Partitioning<_>> = HashSet::from_iter(vec![
        vec![vec![&1], vec![&2, &3]],
        vec![vec![&1, &2], vec![&3]],
        vec![vec![&1, &3], vec![&2]],
    ]);
    assert_eq!(got.len(), want.len());
    let got = HashSet::from_iter(got);
    assert_eq!(got, want);
}
