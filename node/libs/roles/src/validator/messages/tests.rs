use crate::{
    attester::{self, WeightedAttester},
    validator::*,
};
use anyhow::Context as _;
use rand::{prelude::StdRng, Rng, SeedableRng};
use zksync_concurrency::ctx;
use zksync_consensus_crypto::Text;
use zksync_consensus_utils::enum_util::Variant as _;

/// Hardcoded view numbers.
fn views() -> impl Iterator<Item = ViewNumber> {
    [8394532, 2297897, 9089304, 7203483, 9982111]
        .into_iter()
        .map(ViewNumber)
}

/// Hardcoded validator secret keys.
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

/// Hardcoded attester secret keys.
fn attester_keys() -> Vec<attester::SecretKey> {
    [
        "attester:secret:secp256k1:27cb45b1670a1ae8d376a85821d51c7f91ebc6e32788027a84758441aaf0a987",
        "attester:secret:secp256k1:20132edc08a529e927f155e710ae7295a2a0d249f1b1f37726894d1d0d8f0d81",
        "attester:secret:secp256k1:0946901f0a6650284726763b12de5da0f06df0016c8ec2144cf6b1903f1979a6",
    ]
    .iter()
    .map(|raw| Text::new(raw).decode().unwrap())
    .collect()
}

/// Hardcoded validator committee.
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

/// Hardcoded attester committee.
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
fn validator_committee_change_detector() {
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
fn test_rota() {
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
    let rota = LeaderSelectionMode::Rota(want.clone());
    for _ in 0..100 {
        let vn: ViewNumber = rng.gen();
        let pk = &want[vn.0 as usize % want.len()];
        assert_eq!(*pk, committee.view_leader(vn, &rota));
    }
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

    /// Hardcoded genesis with no attesters.
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

    /// Hardcoded genesis with attesters.
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
    #[test]
    fn genesis_hash_change_detector_empty_attesters() {
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
    #[test]
    fn genesis_hash_change_detector_nonempty_attesters() {
        let want: GenesisHash = Text::new(
            "genesis_hash:keccak256:47a52a5491873fa4ceb369a334b4c09833a06bd34718fb22e530ab4d70b4daf7",
        )
        .decode()
        .unwrap();
        assert_eq!(want, genesis_with_attesters().hash());
    }

    #[test]
    fn genesis_verify_leader_pubkey_not_in_committee() {
        let mut rng = StdRng::seed_from_u64(29483920);
        let mut genesis = rng.gen::<GenesisRaw>();
        genesis.leader_selection = LeaderSelectionMode::Sticky(rng.gen());
        let genesis = genesis.with_hash();
        assert!(genesis.verify().is_err())
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

    /// Hardcoded `ReplicaNewView`.
    fn replica_new_view() -> ReplicaNewView {
        ReplicaNewView {
            justification: ProposalJustification::Commit(commit_qc()),
        }
    }

    /// Hardcoded `ReplicaTimeout`
    fn replica_timeout() -> ReplicaTimeout {
        ReplicaTimeout {
            view: view(),
            high_vote: Some(replica_commit()),
            high_qc: Some(commit_qc()),
        }
    }

    /// Hardcoded `TimeoutQC`.
    fn timeout_qc() -> TimeoutQC {
        let mut x = TimeoutQC::new(view());
        let genesis = genesis_empty_attesters();
        let replica_timeout = replica_timeout();
        for k in validator_keys() {
            x.add(&k.sign_msg(replica_timeout.clone()), &genesis)
                .unwrap();
        }
        x
    }

    /// Hardcoded `LeaderProposal`.
    fn leader_proposal() -> LeaderProposal {
        LeaderProposal {
            proposal: block_header(),
            proposal_payload: Some(payload()),
            justification: ProposalJustification::Timeout(timeout_qc()),
        }
    }

    /// Asserts that msg.hash()==hash and that sig is a
    /// valid signature of msg (signed by `keys()[0]`).
    #[track_caller]
    fn change_detector(msg: Msg, hash: &str, sig: &str) {
        let key = validator_keys()[0].clone();

        (|| {
            // Decode hash and signature.
            let hash: MsgHash = Text::new(hash).decode()?;
            let sig: Signature = Text::new(sig).decode()?;

            // Check if msg.hash() is equal to hash.
            if msg.hash() != hash {
                return Err(anyhow::anyhow!("Hash mismatch"));
            }

            // Check if sig is a valid signature of hash.
            sig.verify_hash(&hash, &key.public())?;

            anyhow::Ok(())
        })()
        .with_context(|| {
            format!(
                "\nIntended hash: {:?}\nIntended signature: {:?}",
                msg.hash(),
                key.sign_hash(&msg.hash()),
            )
        })
        .unwrap();
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
    fn replica_new_view_change_detector() {
        change_detector(
            replica_new_view().insert(),
            "validator_msg:keccak256:be1d87c8fcabeb1dd6aebef8564994830f206997d3cf240ad19281a8ef84fbd1",
            "validator:signature:bls12_381:92640683902cd5c092fe8732f556d2bb3e0e209665ba4fe8e6f9ce2572a43700a63e82c3fbfd803fd217b3027ca843ca",
        );
    }

    #[test]
    fn replica_timeout_change_detector() {
        change_detector(
            replica_timeout().insert(),
            "validator_msg:keccak256:c3a2dbbb01fa4884effdbad3cdfea7c03f3fa0d1b8ae15b221ac83e1d7abb9df",
            "validator:signature:bls12_381:acb18d66816a0e7e2c1fbaa2d5045ead1a41eba8692ef0cdbf3f3d3f5a5616fa77903cc45326090cca1468912419d824",
        );
    }

    #[test]
    fn leader_proposal_change_detector() {
        change_detector(
            leader_proposal().insert(),
            "validator_msg:keccak256:ecf7cd55c9b44ae5c361d23067af4f3bf5aa0f4d170f4d33b8ce4214c9963c6b",
            "validator:signature:bls12_381:b650efd18dd800363aafd8ca67e3b6f14963f99b094fb497a3edad56816108a4e835ffcec714320e6fe1b42c30057005",
        );
    }
}
