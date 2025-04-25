use super::*;
use crate::validator::{messages::tests::genesis_v1, ViewNumber};

#[test]
fn test_view_next() {
    let view = View {
        genesis: GenesisHash::default(),
        number: ViewNumber(1),
    };
    let next_view = view.next();
    assert_eq!(next_view.number, ViewNumber(2));
}

#[test]
fn test_view_prev() {
    let view = View {
        genesis: GenesisHash::default(),
        number: ViewNumber(1),
    };
    let prev_view = view.prev();
    assert_eq!(prev_view.unwrap().number, ViewNumber(0));
    let view = View {
        genesis: GenesisHash::default(),
        number: ViewNumber(0),
    };
    let prev_view = view.prev();
    assert!(prev_view.is_none());
}

#[test]
fn test_view_verify() {
    let genesis = genesis_v1();
    let view = View {
        genesis: genesis.hash(),
        number: ViewNumber(1),
    };
    assert!(view.verify(&genesis).is_ok());
    let view = View {
        genesis: GenesisHash::default(),
        number: ViewNumber(1),
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
