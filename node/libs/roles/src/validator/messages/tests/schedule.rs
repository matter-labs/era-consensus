use super::*;
use crate::validator::SecretKey;

fn create_validator(weight: u64) -> ValidatorInfo {
    ValidatorInfo {
        key: SecretKey::generate().public(),
        weight,
        leader: true,
    }
}

/// Checks that the order of validators in a schedule is stable.
#[test]
fn test_schedule_order_change_detector() {
    let schedule = validators_schedule();
    let got: Vec<usize> = validator_keys()
        .iter()
        .map(|k| schedule.index(&k.public()).unwrap())
        .collect();
    assert_eq!(vec![0, 1, 4, 3, 2], got);
}

#[test]
fn test_schedule_new() {
    let validators = vec![create_validator(10), create_validator(20)];
    let schedule = Schedule::new(validators, leader_selection()).unwrap();
    assert_eq!(schedule.len(), 2);
    assert_eq!(schedule.total_weight(), 30);
}

#[test]
fn test_schedule_new_duplicate_validator() {
    let mut validators = vec![create_validator(10), create_validator(20)];
    validators[1].key = validators[0].key.clone();
    let result = Schedule::new(validators, leader_selection());
    assert!(result.is_err());
}

#[test]
fn test_schedule_new_zero_weight() {
    let validators = vec![create_validator(10), create_validator(0)];
    let result = Schedule::new(validators, leader_selection());
    assert!(result.is_err());
}

#[test]
fn test_schedule_weights_overflow_check() {
    let validators: Vec<ValidatorInfo> = [u64::MAX / 5; 6]
        .iter()
        .map(|w| create_validator(*w))
        .collect();
    let result = Schedule::new(validators, leader_selection());
    assert!(result.is_err());
}

#[test]
fn test_schedule_new_empty() {
    let validators = vec![];
    let result = Schedule::new(validators, leader_selection());
    assert!(result.is_err());
}

#[test]
fn test_schedule_contains() {
    let validators = vec![create_validator(10), create_validator(20)];
    let schedule = Schedule::new(validators.clone(), leader_selection()).unwrap();
    assert!(schedule.contains(&validators[0].key));
    assert!(!schedule.contains(&SecretKey::generate().public()));
}

#[test]
fn test_schedule_get() {
    let validator_keys = validator_keys()
        .into_iter()
        .map(|x| x.public())
        .collect::<Vec<_>>();
    let schedule = validators_schedule();
    assert_eq!(schedule.get(0).unwrap().key, validator_keys[0]);
    assert_eq!(schedule.get(1).unwrap().key, validator_keys[1]);
    assert_eq!(schedule.get(2).unwrap().key, validator_keys[4]);
    assert_eq!(schedule.get(3).unwrap().key, validator_keys[3]);
    assert_eq!(schedule.get(4).unwrap().key, validator_keys[2]);
    assert!(schedule.get(5).is_none());
}

#[test]
fn test_schedule_index() {
    let validator_keys = validator_keys()
        .into_iter()
        .map(|x| x.public())
        .collect::<Vec<_>>();
    let schedule = validators_schedule();
    assert_eq!(schedule.index(&validator_keys[0]), Some(0));
    assert_eq!(schedule.index(&validator_keys[1]), Some(1));
    assert_eq!(schedule.index(&validator_keys[4]), Some(2));
    assert_eq!(schedule.index(&validator_keys[3]), Some(3));
    assert_eq!(schedule.index(&validator_keys[2]), Some(4));
    assert_eq!(schedule.index(&SecretKey::generate().public()), None);
}

#[test]
fn test_schedule_quorum_threshold() {
    let validators = vec![create_validator(10), create_validator(20)];
    let schedule = Schedule::new(validators, leader_selection()).unwrap();
    assert_eq!(schedule.quorum_threshold(), 25); // 30 - (30 - 1) / 5
}

#[test]
fn test_schedule_subquorum_threshold() {
    let validators = vec![create_validator(10), create_validator(20)];
    let schedule = Schedule::new(validators, leader_selection()).unwrap();
    assert_eq!(schedule.subquorum_threshold(), 15); // 30 - 3 * (30 - 1) / 5
}

#[test]
fn test_schedule_max_faulty_weight() {
    let validators = vec![create_validator(10), create_validator(20)];
    let schedule = Schedule::new(validators, leader_selection()).unwrap();
    assert_eq!(schedule.max_faulty_weight(), 5); // (30 - 1) / 5
}

#[test]
fn test_leader_selection_round_robin() {
    let schedule = validators_schedule();
    let got: Vec<_> = views()
        .map(|view| {
            let got = schedule.view_leader(view);
            schedule.index(&got).unwrap()
        })
        .collect();
    assert_eq!(vec![2, 3, 4, 4, 1], got);
}

#[test]
fn test_leader_selection_weighted() {
    let leader_selection = LeaderSelection {
        frequency: 1,
        mode: LeaderSelectionMode::Weighted,
    };
    let schedule = Schedule::new(validators(), leader_selection).unwrap();
    let got: Vec<_> = views()
        .map(|view| {
            let got = schedule.view_leader(view);
            schedule.index(&got).unwrap()
        })
        .collect();
    assert_eq!(vec![2, 3, 2, 1, 3], got);
}
