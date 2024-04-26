use crate::validator::*;
use rand::{prelude::StdRng, Rng, SeedableRng};
use zksync_consensus_crypto::Text;
use zksync_concurrency::ctx;
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

/// Hardcoded committee.
fn committee() -> Committee {
    Committee::new(keys().iter().enumerate().map(|(i,key)| WeightedValidator {
        key: key.public(),
        weight: i as u64 + 10,
    })).unwrap()
}

/// Hardcoded payload.
fn payload() -> Payload {
    Payload(
        hex::decode("57b79660558f18d56b5196053f64007030a1cb7eeadb5c32d816b9439f77edf5f6bd9d")
            .unwrap(),
    )
}

/// Checks that the order of validators in a committee is stable.
#[test]
fn committee_change_detector() {
    let want: Vec<PublicKey> = keys().iter().map(|k|k.public()).collect();
    let got: Vec<_> = committee().iter().map(|v|v.key.clone()).collect();
    assert_eq!(want,got);
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

#[test]
fn test_sticky() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let committee = committee();
    let want = committee.get(rng.gen_range(0..committee.len())).unwrap().key.clone();
    let sticky = LeaderSelectionMode::Sticky(want.clone());
    for _ in 0..100 {
        assert_eq!(want, committee.view_leader(rng.gen(), &sticky));
    }
}

/// Checks that leader schedule is stable. 
#[test]
fn roundrobin_change_detector() {
    let committee = committee();
    let rr = LeaderSelectionMode::RoundRobin;
    for (view,want) in [
        (8394534, 0),
        (2297894, 0),
        (9089304, 0),
        (7203484, 0),
        (9982111, 0)
    ] {
        let got = committee.view_leader(ViewNumber(view), &rr);
        assert_eq!(want,committee.index(&got).unwrap());
    }
}

/// Checks that leader schedule is stable. 
#[test]
fn weighted_change_detector() {
    let committee = committee();
    let weighted = LeaderSelectionMode::Weighted;
    for (view,want) in [
        (8394534, 0),
        (2297894, 0),
        (9089304, 0),
        (7203484, 0),
        (9982111, 0)
    ] {
        let got = committee.view_leader(ViewNumber(view), &weighted);
        assert_eq!(want,committee.index(&got).unwrap());
    }
}

mod version1 {
    use super::*;

    /// Hardcoded genesis.
    fn genesis() -> Genesis {
        GenesisRaw {
            chain_id: ChainId(1337),
            fork_number: ForkNumber(402598740274745173),
            first_block: BlockNumber(8902834932452),
            
            protocol_version: ProtocolVersion(1),
            committee: committee(),
            leader_selection: LeaderSelectionMode::Weighted,
        }.with_hash()
    }

    /// Note that genesis is NOT versioned by ProtocolVersion.
    /// Even if it was, ALL versions of genesis need to be supported FOREVER,
    /// unless we introduce dynamic regenesis.
    #[test]
    fn genesis_hash_change_detector() {
        let want: GenesisHash = Text::new(
            "genesis_hash:keccak256:3fc736e5f69784be02ec83ff5f91414ee6b44e545b68eac7f54089bb63085b02",
        )
        .decode()
        .unwrap();
        assert_eq!(want, genesis().hash());
    }

    #[test]
    fn genesis_verify_leader_pubkey_not_in_committee() {
        let mut rng = StdRng::seed_from_u64(29483920);
        let mut genesis = rng.gen::<GenesisRaw>();
        genesis.leader_selection = LeaderSelectionMode::Sticky(rng.gen());
        let genesis = genesis.with_hash();
        assert!(genesis.verify().is_err())
    }

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
            genesis: genesis().hash(),
            number: ViewNumber(9136573498460759103),
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
        let genesis = genesis();
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
        let genesis = genesis();
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

    #[test]
    fn replica_commit_change_detector() {
        change_detector(
            replica_commit().insert(),
            "validator_msg:keccak256:bc629a46e67d0ceef09f898afe7c773b010f78f474452226364deb12f26bff59",
            "validator:signature:bn254:09dca52611cf60eba99293a1ffec853ba65370b5c6727c5009748f7f59fefabd",
        );
    }

    #[test]
    fn leader_commit_change_detector() {
        change_detector(
            leader_commit().insert(),
            "validator_msg:keccak256:340c4f1d075d070a8bbde198c777f89e3c025b8e14e1d32328a52b694d7fb7da",
            "validator:signature:bn254:08729ad003eee453696b72e56d2a75124730ff17376fca7099a21f32ff1b265a",
        );
    }

    #[test]
    fn replica_prepare_change_detector() {
        change_detector(
            replica_prepare().insert(),
            "validator_msg:keccak256:361382ac2738d16f2b013f8674550970b8a5d79ab92eb1a437df2e478a0bbf46",
            "validator:signature:bn254:8bd0a2f83e7fc0321a9d487266ca3e7ad4f717f8bf314ce1bb858f7235f84914",
        );
    }

    #[test]
    fn leader_prepare_change_detector() {
        change_detector(
            leader_prepare().insert(),
            "validator_msg:keccak256:e29ae451d7bd6a72e1cccb5666fb66ddbd893e506d91e1985e9723a65bd9298b",
            "validator:signature:bn254:92a42139359540383f90f01f7f69907a4f4e04f1753ffd857266ed4c157f7fa9",
        );
    }
}
