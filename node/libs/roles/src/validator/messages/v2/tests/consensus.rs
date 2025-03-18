use super::*;
use crate::validator::{
    messages::tests::{genesis_v2, validator_committee},
    ViewNumber,
};

#[test]
fn test_view_next() {
    let view = View {
        genesis: GenesisHash::default(),
        number: ViewNumber(1),
        epoch: EpochNumber(0),
    };

    let next_view = view.next_view();
    assert_eq!(next_view.number, ViewNumber(2));

    let next_epoch = view.next_epoch();
    assert_eq!(next_epoch.epoch, EpochNumber(1));
}

#[test]
fn test_view_prev() {
    let view = View {
        genesis: GenesisHash::default(),
        number: ViewNumber(1),
        epoch: EpochNumber(2),
    };

    let prev_view = view.prev_view();
    assert_eq!(prev_view.unwrap().number, ViewNumber(0));

    let prev_epoch = view.prev_epoch();
    assert_eq!(prev_epoch.unwrap().epoch, EpochNumber(1));

    let view = View {
        genesis: GenesisHash::default(),
        number: ViewNumber(0),
        epoch: EpochNumber(0),
    };

    let prev_view = view.prev_view();
    assert!(prev_view.is_none());

    let prev_epoch = view.prev_epoch();
    assert!(prev_epoch.is_none());
}

#[test]
fn test_view_verify() {
    let genesis = genesis_v2();

    let view = View {
        genesis: genesis.hash(),
        number: ViewNumber(1),
        epoch: EpochNumber(0),
    };
    assert!(view.verify(&genesis).is_ok());

    let view = View {
        genesis: GenesisHash::default(),
        number: ViewNumber(1),
        epoch: EpochNumber(0),
    };
    assert!(view.verify(&genesis).is_err());
}

#[test]
fn test_signers_new() {
    let signers = Signers::new(10);
    assert_eq!(signers.len(), 10);
    assert!(signers.is_empty());
}

#[test]
fn test_signers_count() {
    let mut signers = Signers::new(10);
    signers.0.set(0, true);
    signers.0.set(1, true);
    assert_eq!(signers.count(), 2);
}

#[test]
fn test_signers_empty() {
    let mut signers = Signers::new(10);
    assert!(signers.is_empty());
    signers.0.set(1, true);
    assert!(!signers.is_empty());
    signers.0.set(1, false);
    assert!(signers.is_empty());
}

#[test]
fn test_signers_bitor_assign() {
    let mut signers1 = Signers::new(10);
    let mut signers2 = Signers::new(10);
    signers1.0.set(0, true);
    signers1.0.set(3, true);
    signers2.0.set(1, true);
    signers2.0.set(3, true);
    signers1 |= &signers2;
    assert_eq!(signers1.count(), 3);
}

#[test]
fn test_signers_bitand_assign() {
    let mut signers1 = Signers::new(10);
    let mut signers2 = Signers::new(10);
    signers1.0.set(0, true);
    signers1.0.set(3, true);
    signers2.0.set(1, true);
    signers2.0.set(3, true);
    signers1 &= &signers2;
    assert_eq!(signers1.count(), 1);
}

#[test]
fn test_signers_weight() {
    let committee = validator_committee();
    let mut signers = Signers::new(5);
    signers.0.set(1, true);
    signers.0.set(2, true);
    signers.0.set(4, true);
    assert_eq!(signers.weight(&committee), 37);
}

#[test]
fn test_leader_selection_round_robin() {
    let committee = validator_committee();
    let mode = LeaderSelectionMode::RoundRobin;
    let got: Vec<_> = views()
        .map(|view| {
            let got = mode.view_leader(view, &committee);
            committee.index(&got).unwrap()
        })
        .collect();
    assert_eq!(vec![2, 3, 4, 4, 1], got);
}

#[test]
fn test_leader_selection_weighted() {
    let committee = validator_committee();
    let mode = LeaderSelectionMode::Weighted;
    let got: Vec<_> = views()
        .map(|view| {
            let got = mode.view_leader(view, &committee);
            committee.index(&got).unwrap()
        })
        .collect();
    assert_eq!(vec![2, 3, 2, 1, 3], got);
}

#[test]
fn test_leader_selection_sticky() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let committee = validator_committee();
    let want = committee
        .get(rng.gen_range(0..committee.len()))
        .unwrap()
        .key
        .clone();
    let mode = LeaderSelectionMode::Sticky(want.clone());
    for _ in 0..100 {
        assert_eq!(want, mode.view_leader(rng.gen(), &committee));
    }
}

#[test]
fn test_leader_selection_rota() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let committee = validator_committee();
    let mut want = Vec::new();
    for _ in 0..3 {
        want.push(
            committee
                .get(rng.gen_range(0..committee.len()))
                .unwrap()
                .key
                .clone(),
        );
    }
    let mode = LeaderSelectionMode::Rota(want.clone());
    for _ in 0..100 {
        let vn: ViewNumber = rng.gen();
        let pk = &want[vn.0 as usize % want.len()];
        assert_eq!(*pk, mode.view_leader(vn, &committee));
    }
}
