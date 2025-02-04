use crate::validator;
use crate::validator::messages::testonly;
use crate::validator::messages::{Committee, LeaderSelectionMode, WeightedValidator};
use rand::Rng;
use zksync_concurrency::ctx;

/// Checks that the order of validators in a committee is stable.
#[test]
fn test_committee_order_change_detector() {
    let committee = testonly::validator_committee();
    let got: Vec<usize> = testonly::validator_keys()
        .iter()
        .map(|k| committee.index(&k.public()).unwrap())
        .collect();
    assert_eq!(vec![0, 1, 4, 3, 2], got);
}

fn create_validator(weight: u64) -> WeightedValidator {
    WeightedValidator {
        key: validator::SecretKey::generate().public(),
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
    assert!(!committee.contains(&validator::SecretKey::generate().public()));
}

#[test]
fn test_committee_get() {
    let validators = testonly::validator_keys()
        .into_iter()
        .map(|x| x.public())
        .collect::<Vec<_>>();
    let committee = testonly::validator_committee();
    assert_eq!(committee.get(0).unwrap().key, validators[0]);
    assert_eq!(committee.get(1).unwrap().key, validators[1]);
    assert_eq!(committee.get(2).unwrap().key, validators[4]);
    assert_eq!(committee.get(3).unwrap().key, validators[3]);
    assert_eq!(committee.get(4).unwrap().key, validators[2]);
    assert!(committee.get(5).is_none());
}

#[test]
fn test_committee_index() {
    let validators = testonly::validator_keys()
        .into_iter()
        .map(|x| x.public())
        .collect::<Vec<_>>();
    let committee = testonly::validator_committee();
    assert_eq!(committee.index(&validators[0]), Some(0));
    assert_eq!(committee.index(&validators[1]), Some(1));
    assert_eq!(committee.index(&validators[4]), Some(2));
    assert_eq!(committee.index(&validators[3]), Some(3));
    assert_eq!(committee.index(&validators[2]), Some(4));
    assert_eq!(
        committee.index(&validator::SecretKey::generate().public()),
        None
    );
}

#[test]
fn test_committee_view_leader_round_robin() {
    let committee = testonly::validator_committee();
    let mode = LeaderSelectionMode::RoundRobin;
    let got: Vec<_> = views()
        .map(|view| {
            let got = committee.view_leader(view, &mode);
            committee.index(&got).unwrap()
        })
        .collect();
    assert_eq!(vec![2, 3, 4, 4, 1], got);
}

#[test]
fn test_committee_view_leader_weighted() {
    let committee = testonly::validator_committee();
    let mode = LeaderSelectionMode::Weighted;
    let got: Vec<_> = views()
        .map(|view| {
            let got = committee.view_leader(view, &mode);
            committee.index(&got).unwrap()
        })
        .collect();
    assert_eq!(vec![2, 3, 2, 1, 3], got);
}

#[test]
fn test_committee_view_leader_sticky() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let committee = testonly::validator_committee();
    let want = committee
        .get(rng.gen_range(0..committee.len()))
        .unwrap()
        .key
        .clone();
    let sticky = LeaderSelectionMode::Sticky(want.clone());
    for _ in 0..100 {
        assert_eq!(want, committee.view_leader(rng.gen(), &sticky));
    }
}

#[test]
fn test_committee_view_leader_rota() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let committee = testonly::validator_committee();
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
    let rota = LeaderSelectionMode::Rota(want.clone());
    for _ in 0..100 {
        let vn: ViewNumber = rng.gen();
        let pk = &want[vn.0 as usize % want.len()];
        assert_eq!(*pk, committee.view_leader(vn, &rota));
    }
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

#[test]
fn test_committee_weight() {
    let committee = testonly::validator_committee();
    let mut signers = Signers::new(5);
    signers.0.set(1, true);
    signers.0.set(2, true);
    signers.0.set(4, true);
    assert_eq!(committee.weight(&signers), 37);
}
