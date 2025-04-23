use super::*;
use crate::validator::{messages::tests::validator_committee, SecretKey};

/// Checks that the order of validators in a committee is stable.
#[test]
fn test_committee_order_change_detector() {
    let committee = validator_committee();
    let got: Vec<usize> = validator_keys()
        .iter()
        .map(|k| committee.index(&k.public()).unwrap())
        .collect();
    assert_eq!(vec![0, 1, 4, 3, 2], got);
}

fn create_validator(weight: u64) -> WeightedValidator {
    WeightedValidator {
        key: SecretKey::generate().public(),
        weight,
    }
}

#[test]
fn test_committee_new() {
    let validators = vec![create_validator(10), create_validator(20)];
    let committee = Committee::new(validators).unwrap();
    assert_eq!(committee.len(), 2);
    assert_eq!(committee.total_weight(), 30);
}

#[test]
fn test_committee_new_duplicate_validator() {
    let mut validators = vec![create_validator(10), create_validator(20)];
    validators[1].key = validators[0].key.clone();
    let result = Committee::new(validators);
    assert!(result.is_err());
}

#[test]
fn test_committee_new_zero_weight() {
    let validators = vec![create_validator(10), create_validator(0)];
    let result = Committee::new(validators);
    assert!(result.is_err());
}

#[test]
fn test_committee_weights_overflow_check() {
    let validators: Vec<WeightedValidator> = [u64::MAX / 5; 6]
        .iter()
        .map(|w| create_validator(*w))
        .collect();
    let result = Committee::new(validators);
    assert!(result.is_err());
}

#[test]
fn test_committee_new_empty() {
    let validators = vec![];
    let result = Committee::new(validators);
    assert!(result.is_err());
}

#[test]
fn test_committee_contains() {
    let validators = vec![create_validator(10), create_validator(20)];
    let committee = Committee::new(validators.clone()).unwrap();
    assert!(committee.contains(&validators[0].key));
    assert!(!committee.contains(&SecretKey::generate().public()));
}

#[test]
fn test_committee_get() {
    let validators = validator_keys()
        .into_iter()
        .map(|x| x.public())
        .collect::<Vec<_>>();
    let committee = validator_committee();
    assert_eq!(committee.get(0).unwrap().key, validators[0]);
    assert_eq!(committee.get(1).unwrap().key, validators[1]);
    assert_eq!(committee.get(2).unwrap().key, validators[4]);
    assert_eq!(committee.get(3).unwrap().key, validators[3]);
    assert_eq!(committee.get(4).unwrap().key, validators[2]);
    assert!(committee.get(5).is_none());
}

#[test]
fn test_committee_index() {
    let validators = validator_keys()
        .into_iter()
        .map(|x| x.public())
        .collect::<Vec<_>>();
    let committee = validator_committee();
    assert_eq!(committee.index(&validators[0]), Some(0));
    assert_eq!(committee.index(&validators[1]), Some(1));
    assert_eq!(committee.index(&validators[4]), Some(2));
    assert_eq!(committee.index(&validators[3]), Some(3));
    assert_eq!(committee.index(&validators[2]), Some(4));
    assert_eq!(committee.index(&SecretKey::generate().public()), None);
}

#[test]
fn test_committee_quorum_threshold() {
    let validators = vec![create_validator(10), create_validator(20)];
    let committee = Committee::new(validators).unwrap();
    assert_eq!(committee.quorum_threshold(), 25); // 30 - (30 - 1) / 5
}

#[test]
fn test_committee_subquorum_threshold() {
    let validators = vec![create_validator(10), create_validator(20)];
    let committee = Committee::new(validators).unwrap();
    assert_eq!(committee.subquorum_threshold(), 15); // 30 - 3 * (30 - 1) / 5
}

#[test]
fn test_committee_max_faulty_weight() {
    let validators = vec![create_validator(10), create_validator(20)];
    let committee = Committee::new(validators).unwrap();
    assert_eq!(committee.max_faulty_weight(), 5); // (30 - 1) / 5
}
