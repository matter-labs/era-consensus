use crate::attester::{self, WeightedAttester};
use crate::validator::*;
use anyhow::Context as _;
use rand::{prelude::StdRng, Rng, SeedableRng};
use zksync_concurrency::ctx;
use zksync_consensus_crypto::Text;
use zksync_consensus_utils::enum_util::Variant as _;

/// Hardcoded secret keys.
fn validator_keys() -> Vec<SecretKey> {
    [
        "validator:secret:bls12_381:27cb45b1670a1ae8d376a85821d51c7f91ebc6e32788027a84758441aaf0a987",
        "validator:secret:bls12_381:20132edc08a529e927f155e710ae7295a2a0d249f1b1f37726894d1d0d8f0d81",
        "validator:secret:bls12_381:0946901f0a6650284726763b12de5da0f06df0016c8ec2144cf6b1903f1979a6",
        "validator:secret:bls12_381:3143a64c079b2f50545288d7c9b282281e05c97ac043228830a9660ddd63fea3",
        "validator:secret:bls12_381:5512f40d33844c1c8107aa630af764005ab6e13f6bf8edb59b4ca3683727e619",
    ]
    .iter()
    .map(|raw| Text::new(raw).decode().unwrap())
    .collect()
}

fn attester_keys() -> Vec<attester::SecretKey> {
    [
        "attester:secret:bn254:27cb45b1670a1ae8d376a85821d51c7f91ebc6e32788027a84758441aaf0a987",
        "attester:secret:bn254:20132edc08a529e927f155e710ae7295a2a0d249f1b1f37726894d1d0d8f0d81",
        "attester:secret:bn254:0946901f0a6650284726763b12de5da0f06df0016c8ec2144cf6b1903f1979a6",
    ]
    .iter()
    .map(|raw| Text::new(raw).decode().unwrap())
    .collect()
}

/// Hardcoded committee.
fn validator_committee() -> Committee {
    Committee::new(
        validator_keys()
            .iter()
            .enumerate()
            .map(|(i, key)| WeightedValidator {
                key: key.public(),
                weight: i as u64 + 10,
            }),
    )
    .unwrap()
}

fn attester_committee() -> attester::Committee {
    attester::Committee::new(
        attester_keys()
            .iter()
            .enumerate()
            .map(|(i, key)| WeightedAttester {
                key: key.public(),
                weight: i as u64 + 10,
            }),
    )
    .unwrap()
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
    let committee = validator_committee();
    let got: Vec<usize> = validator_keys()
        .iter()
        .map(|k| committee.index(&k.public()).unwrap())
        .collect();
    assert_eq!(vec![0, 1, 4, 3, 2], got);
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
    let sticky = LeaderSelectionMode::Sticky(want.clone());
    for _ in 0..100 {
        let vn: ViewNumber = rng.gen();
        let pk = &want[vn.0 as usize % want.len()];
        assert_eq!(*pk, committee.view_leader(vn, &sticky));
    }
}

/// Hardcoded view numbers.
fn views() -> impl Iterator<Item = ViewNumber> {
    [8394532, 2297897, 9089304, 7203483, 9982111]
        .into_iter()
        .map(ViewNumber)
}

/// Checks that leader schedule is stable.
#[test]
fn roundrobin_change_detector() {
    let committee = validator_committee();
    let mode = LeaderSelectionMode::RoundRobin;
    let got: Vec<_> = views()
        .map(|view| {
            let got = committee.view_leader(view, &mode);
            committee.index(&got).unwrap()
        })
        .collect();
    assert_eq!(vec![2, 2, 4, 3, 1], got);
}

/// Checks that leader schedule is stable.
#[test]
fn weighted_change_detector() {
    let committee = validator_committee();
    let mode = LeaderSelectionMode::Weighted;
    let got: Vec<_> = views()
        .map(|view| {
            let got = committee.view_leader(view, &mode);
            committee.index(&got).unwrap()
        })
        .collect();
    assert_eq!(vec![4, 2, 2, 2, 1], got);
}

mod version1 {
    use super::*;

    /// Hardcoded genesis.
    fn genesis_empty_attesters() -> Genesis {
        GenesisRaw {
            chain_id: ChainId(1337),
            fork_number: ForkNumber(402598740274745173),
            first_block: BlockNumber(8902834932452),

            protocol_version: ProtocolVersion(1),
            validators: validator_committee(),
            attesters: None,
            leader_selection: LeaderSelectionMode::Weighted,
        }
        .with_hash()
    }

    /// Hardcoded genesis.
    fn genesis_with_attesters() -> Genesis {
        GenesisRaw {
            chain_id: ChainId(1337),
            fork_number: ForkNumber(402598740274745173),
            first_block: BlockNumber(8902834932452),

            protocol_version: ProtocolVersion(1),
            validators: validator_committee(),
            attesters: attester_committee().into(),
            leader_selection: LeaderSelectionMode::Weighted,
        }
        .with_hash()
    }

    /// Note that genesis is NOT versioned by ProtocolVersion.
    /// Even if it was, ALL versions of genesis need to be supported FOREVER,
    /// unless we introduce dynamic regenesis.
    /// FIXME: This fails with the new attester committee.
    #[test]
    fn genesis_hash_change_detector() {
        let want: GenesisHash = Text::new(
            "genesis_hash:keccak256:13a16cfa758c6716b4c4d40a5fe71023a016c7507b7893c7dc775f4420fc5d61",
        )
        .decode()
        .unwrap();
        assert_eq!(want, genesis_empty_attesters().hash());
    }

    /// Note that genesis is NOT versioned by ProtocolVersion.
    /// Even if it was, ALL versions of genesis need to be supported FOREVER,
    /// unless we introduce dynamic regenesis.
    /// FIXME: This fails with the new attester committee.
    #[test]
    fn genesis_hash_change_detector_2() {
        let want: GenesisHash = Text::new(
            "genesis_hash:keccak256:63d6562ea2a27069e64a4005d1aef446907db945d85e06323296d2c0f8336c65",
        )
        .decode()
        .unwrap();
        assert_eq!(want, genesis_with_attesters().hash());
    }

    #[test]
    fn genesis_verify_leader_pubkey_not_in_committee() {
        let mut rng = StdRng::seed_from_u64(29483920);
        let mut genesis = rng.gen::<GenesisRaw>();
        genesis.leader_selection = LeaderSelectionMode::Sticky(vec![rng.gen()]);
        let genesis = genesis.with_hash();
        assert!(genesis.verify().is_err())
    }

    /// asserts that msg.hash()==hash and that sig is a
    /// valid signature of msg (signed by `keys()[0]`).
    #[track_caller]
    fn change_detector(msg: Msg, hash: &str, sig: &str) {
        let key = validator_keys()[0].clone();
        (|| {
            let hash: MsgHash = Text::new(hash).decode()?;
            let sig: Signature = Text::new(sig).decode()?;
            sig.verify_hash(&hash, &key.public())?;
            anyhow::Ok(())
        })()
        .with_context(|| format!("\n{:?},\n{:?}", msg.hash(), key.sign_hash(&msg.hash()),))
        .unwrap();
    }

    /// Hardcoded view.
    fn view() -> View {
        View {
            genesis: genesis_empty_attesters().hash(),
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
        let genesis = genesis_empty_attesters();
        let replica_commit = replica_commit();
        let mut x = CommitQC::new(replica_commit.clone(), &genesis);
        for k in validator_keys() {
            x.add(&k.sign_msg(replica_commit.clone()), &genesis)
                .unwrap();
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
        let genesis = genesis_empty_attesters();
        let replica_prepare = replica_prepare();
        for k in validator_keys() {
            x.add(&k.sign_msg(replica_prepare.clone()), &genesis)
                .unwrap();
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
            "validator_msg:keccak256:2ec798684e539d417fac1caba74ed1a27a033bc18058ba0a4632f6bb0ae4fe1c",
            "validator:signature:bls12_381:8de9ad850d78eb4f918c8c3a02310be49fc9ac35f2b1fdd6489293db1d5128f0d4c8389674e6bc2eee4c6e16f58e0b51",
        );
    }

    #[test]
    fn leader_commit_change_detector() {
        change_detector(
            leader_commit().insert(),
            "validator_msg:keccak256:53b8d7dc77a5ba8b81cd7b46ddc19c224ef46245c26cb7ae12239acc1bf86eda",
            "validator:signature:bls12_381:81a154a93a8b607031319915728be97c03c3014a4746050f7a32cde98cabe4fbd2b6d6b79400601a71f50350842d1d64",
        );
    }

    #[test]
    fn replica_prepare_change_detector() {
        change_detector(
            replica_prepare().insert(),
            "validator_msg:keccak256:700cf26d50f463cfa908f914d1febb1cbd00ee9d3a691b644f49146ed3e6ac40",
            "validator:signature:bls12_381:a7cbdf9b8d13ebc39f4a13d654ec30acccd247d46fc6121eb1220256cfc212b418aac85400176e8797d8eb91aa70ae78",
        );
    }

    #[test]
    fn leader_prepare_change_detector() {
        change_detector(
            leader_prepare().insert(),
            "validator_msg:keccak256:aaaaa6b7b232ef5b7c797953ce2a042c024137d7b8f449a1ad8a535730bc269b",
            "validator:signature:bls12_381:a1926f460fa63470544cc9213e6378f45d75dff3055924766a81ff696a6a6e85ee583707911bb7fef4d1f74b7b28132f",
        );
    }
}
