use crate::validator::*;
use rand::{prelude::StdRng, Rng, SeedableRng};
use zksync_consensus_crypto::Text;
use zksync_consensus_utils::enum_util::Variant as _;

/// Hardcoded secret keys.
fn keys() -> Vec<SecretKey> {
    [
        "validator:secret:bls12_381:27cb45b1670a1ae8d376a85821d51c7f91ebc6e32788027a84758441aaf0a987",
        "validator:secret:bls12_381:20132edc08a529e927f155e710ae7295a2a0d249f1b1f37726894d1d0d8f0d81",
        "validator:secret:bls12_381:0946901f0a6650284726763b12de5da0f06df0016c8ec2144cf6b1903f1979a6",
    ]
    .iter()
    .map(|raw| Text::new(raw).decode().unwrap())
    .collect()
}

/// Hardcoded payload.
fn payload() -> Payload {
    Payload(
        hex::decode("57b79660558f18d56b5196053f64007030a1cb7eeadb5c32d816b9439f77edf5f6bd9d")
            .unwrap(),
    )
}

/// Hardcoded fork.
fn fork() -> Fork {
    Fork {
        number: ForkNumber(402598740274745173),
        first_block: BlockNumber(8902834932452),
    }
}

/// Hardcoded v0 genesis.
fn genesis_v0() -> Genesis {
    Genesis {
        validators: Committee::new(keys().iter().map(|k| WeightedValidator {
            key: k.public(),
            weight: 1,
        }))
        .unwrap(),
        fork: fork(),
        version: GenesisVersion(0),
        leader_selection: None,
    }
}

/// Hardcoded v1 genesis.
fn genesis_v1() -> Genesis {
    Genesis {
        validators: Committee::new(keys().iter().map(|k| WeightedValidator {
            key: k.public(),
            weight: 1,
        }))
        .unwrap(),
        fork: fork(),
        version: GenesisVersion(1),
        leader_selection: Some(LeaderSelectionMode::Weighted),
    }
}

#[test]
fn payload_hash_change_detector() {
    let want: PayloadHash = Text::new(
        "payload:keccak256:ba8ffff2526cae27a9e8e014749014b08b80e01905c8b769159d02d6579d9b83",
    )
    .decode()
    .unwrap();
    assert_eq!(want, payload().hash());
}

/// Note that genesis is NOT versioned by ProtocolVersion.
/// Even if it was, ALL versions of genesis need to be supported FOREVER,
/// unless we introduce dynamic regenesis.
#[ignore]
#[test]
fn genesis_v0_hash_change_detector() {
    let want: GenesisHash = Text::new(
        "genesis_hash:keccak256:9c9bfa303e8d2d451a7fadd327f5f1b957a37c84d7b27b9e1cf7b92fd83af7ae",
    )
    .decode()
    .unwrap();
    assert_eq!(want, genesis_v0().hash());
}

#[ignore]
#[test]
fn genesis_v1_hash_change_detector() {
    let want: GenesisHash = Text::new(
        "genesis_hash:keccak256:3fc736e5f69784be02ec83ff5f91414ee6b44e545b68eac7f54089bb63085b02",
    )
    .decode()
    .unwrap();
    assert_eq!(want, genesis_v1().hash());
}

#[test]
fn genesis_verify_leader_pubkey_not_in_committee() {
    let mut rng = StdRng::seed_from_u64(29483920);
    let mut genesis = rng.gen::<Genesis>();
    genesis.leader_selection = Some(LeaderSelectionMode::Sticky(rng.gen()));
    assert!(genesis.verify().is_err())
}

#[test]
fn leader_selection_mode_roundrobin() {
    let validators = keys().into_iter().map(|k| WeightedValidator {
        key: k.public(),
        weight: 10,
    });
    let committee = Committee::new(validators).unwrap();
    let leader_selection = LeaderSelectionMode::RoundRobin;

    let mut rng = StdRng::seed_from_u64(29483920);
    for _ in 0..100 {
        let view_number: u64 = rng.gen();
        let leader = committee.view_leader(ViewNumber(view_number), leader_selection.clone());
        let leader_index = view_number as usize % committee.len();
        assert_eq!(leader, committee.get(leader_index).unwrap().key);
    }
}

#[test]
fn leader_selection_mode_sticky() {
    let validators = keys().into_iter().map(|k| WeightedValidator {
        key: k.public(),
        weight: 10,
    });
    let committee = Committee::new(validators).unwrap();
    let validator = committee.get(committee.len() - 1).unwrap().key.clone();
    let leader_selection = LeaderSelectionMode::Sticky(validator.clone());

    let mut rng = StdRng::seed_from_u64(29483920);
    for _ in 0..100 {
        let view_number: u64 = rng.gen();
        let leader = committee.view_leader(ViewNumber(view_number), leader_selection.clone());
        assert_eq!(leader, validator);
    }
}

#[test]
fn leader_selection_mode_weighted() {
    let weight = 1000;
    let validators = keys().into_iter().map(|k| WeightedValidator {
        key: k.public(),
        weight,
    });
    let committee = Committee::new(validators).unwrap();
    let leader_selection = LeaderSelectionMode::Weighted;

    let mut rng = StdRng::seed_from_u64(29483920);
    for _ in 0..100 {
        let view_number: u64 = rng.gen();
        let eligibility = leader_weighted_eligibility(view_number, committee.total_weight());
        let leader = committee.view_leader(ViewNumber(view_number), leader_selection.clone());
        let leader_index = eligibility / weight;
        assert_eq!(leader, committee.get(leader_index as usize).unwrap().key);
    }
}

mod version1 {
    const VERSION: ProtocolVersion = ProtocolVersion(1);
    use super::*;

    /// asserts that msg.hash()==hash and that sig is a
    /// valid signature of msg (signed by `keys()[0]`).
    #[track_caller]
    fn change_detector(msg: Msg, hash: &str, sig: &str) {
        let hash: MsgHash = Text::new(hash).decode().unwrap();
        assert!(hash == msg.hash(), "bad hash, want {:?}", msg.hash());
        let sig: Signature = Text::new(sig).decode().unwrap();
        let key = keys()[0].clone();
        assert!(
            sig.verify_hash(&hash, &key.public()).is_ok(),
            "bad signature, want {:?}",
            key.sign_hash(&hash),
        );
    }

    /// Hardcoded view.
    fn view() -> View {
        View {
            protocol_version: VERSION,
            number: ViewNumber(9136573498460759103),
            fork: fork().number,
        }
    }

    /// Hardcoded `BlockHeader`.
    fn block_header() -> BlockHeader {
        BlockHeader {
            number: BlockNumber(772839452345),
            payload: payload().hash(),
        }
    }

    /// Hardcoded `ReplicaCommit`.
    fn replica_commit() -> ReplicaCommit {
        ReplicaCommit {
            view: view(),
            proposal: block_header(),
        }
    }

    /// Hardcoded `CommitQC`.
    fn commit_qc() -> CommitQC {
        let genesis = genesis_v1();
        let replica_commit = replica_commit();
        let mut x = CommitQC::new(replica_commit.clone(), &genesis);
        for k in keys() {
            x.add(&k.sign_msg(replica_commit.clone()), &genesis);
        }
        x
    }

    /// Hardcoded `LeaderCommit`.
    fn leader_commit() -> LeaderCommit {
        LeaderCommit {
            justification: commit_qc(),
        }
    }

    /// Hardcoded `ReplicaPrepare`
    fn replica_prepare() -> ReplicaPrepare {
        ReplicaPrepare {
            view: view(),
            high_vote: Some(replica_commit()),
            high_qc: Some(commit_qc()),
        }
    }

    /// Hardcoded `PrepareQC`.
    fn prepare_qc() -> PrepareQC {
        let mut x = PrepareQC::new(view());
        let genesis = genesis_v1();
        let replica_prepare = replica_prepare();
        for k in keys() {
            x.add(&k.sign_msg(replica_prepare.clone()), &genesis);
        }
        x
    }

    /// Hardcoded `LeaderPrepare`.
    fn leader_prepare() -> LeaderPrepare {
        LeaderPrepare {
            proposal: block_header(),
            proposal_payload: Some(payload()),
            justification: prepare_qc(),
        }
    }

    #[ignore]
    #[test]
    fn replica_commit_change_detector() {
        change_detector(
            replica_commit().insert(),
            "validator_msg:keccak256:bc629a46e67d0ceef09f898afe7c773b010f78f474452226364deb12f26bff59",
            "validator:signature:bn254:09dca52611cf60eba99293a1ffec853ba65370b5c6727c5009748f7f59fefabd",
        );
    }

    #[ignore]
    #[test]
    fn leader_commit_change_detector() {
        change_detector(
            leader_commit().insert(),
            "validator_msg:keccak256:340c4f1d075d070a8bbde198c777f89e3c025b8e14e1d32328a52b694d7fb7da",
            "validator:signature:bn254:08729ad003eee453696b72e56d2a75124730ff17376fca7099a21f32ff1b265a",
        );
    }

    #[ignore]
    #[test]
    fn replica_prepare_change_detector() {
        change_detector(
            replica_prepare().insert(),
            "validator_msg:keccak256:361382ac2738d16f2b013f8674550970b8a5d79ab92eb1a437df2e478a0bbf46",
            "validator:signature:bn254:8bd0a2f83e7fc0321a9d487266ca3e7ad4f717f8bf314ce1bb858f7235f84914",
        );
    }

    #[ignore]
    #[test]
    fn leader_prepare_change_detector() {
        change_detector(
            leader_prepare().insert(),
            "validator_msg:keccak256:e29ae451d7bd6a72e1cccb5666fb66ddbd893e506d91e1985e9723a65bd9298b",
            "validator:signature:bn254:92a42139359540383f90f01f7f69907a4f4e04f1753ffd857266ed4c157f7fa9",
        );
    }
}
