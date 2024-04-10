use crate::validator::*;
use zksync_consensus_crypto::Text;
use zksync_consensus_utils::enum_util::Variant as _;

/// Hardcoded secret keys.
fn keys() -> Vec<SecretKey> {
    [
        "validator:secret:bn254:27cb45b1670a1ae8d376a85821d51c7f91ebc6e32788027a84758441aaf0a987",
        "validator:secret:bn254:20132edc08a529e927f155e710ae7295a2a0d249f1b1f37726894d1d0d8f0d81",
        "validator:secret:bn254:0946901f0a6650284726763b12de5da0f06df0016c8ec2144cf6b1903f1979a6",
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

/// Hardcoded genesis.
fn genesis() -> Genesis {
    Genesis {
        validators: ValidatorSet::new(keys().iter().map(|k| k.public())).unwrap(),
        fork: fork(),
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
#[test]
fn genesis_hash_change_detector() {
    let want: GenesisHash = Text::new(
        "genesis_hash:keccak256:9c9bfa303e8d2d451a7fadd327f5f1b957a37c84d7b27b9e1cf7b92fd83af7ae",
    )
    .decode()
    .unwrap();
    assert_eq!(want, genesis().hash());
}

mod version1 {
    const VERSION: ProtocolVersion = ProtocolVersion(1);
    use super::*;

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
        let want: MsgHash = Text::new("validator_msg:keccak256:3538aa25b136b1ec7a8c9f8ac660b2cfabf06a6af35620c6d06c48200a6e43d4").decode().unwrap();
        assert_eq!(want, replica_commit().insert().hash());
        let sig: Signature = Text::new("validator:signature:bn254:acf1d30e191be77f7d49ae959403b650cb8d2be6472d0296b31912d63ae74ea9").decode().unwrap();
        sig.verify_hash(&want, &keys()[0].public()).unwrap();
    }

    #[test]
    fn leader_commit_change_detector() {
        let want: MsgHash = Text::new("validator_msg:keccak256:f17469f8ab95ae08c5bb383ccdceb9b685cb4c8fc6e75b7eefef7ea5c419e386").decode().unwrap();
        assert_eq!(want, leader_commit().insert().hash());
        let sig: Signature = Text::new("validator:signature:bn254:8663ff8f097e51d259f41450652fc936890d9a3592de07c5f81c0be57cb301cd").decode().unwrap();
        sig.verify_hash(&want, &keys()[0].public()).unwrap();
    }

    #[test]
    fn replica_prepare_change_detector() {
        let want : MsgHash = Text::new("validator_msg:keccak256:2f3da9f4ad132e333a383bf97aa8c0c82fd302289b33f6f57e9bcd1a53b0b75d").decode().unwrap();
        assert_eq!(want, replica_prepare().insert().hash());
        let sig: Signature = Text::new("validator:signature:bn254:9a2f2f3a56ac385d8b3e5fb3db563d6785bffc9a73f2babe858d24233098f723").decode().unwrap();
        sig.verify_hash(&want, &keys()[0].public()).unwrap();
    }

    #[test]
    fn leader_prepare_change_detector() {
        let want : MsgHash = Text::new("validator_msg:keccak256:2a8389574a692a48525c43b5a9a444da7bade1b4b4d95ccec7d9fb3d731a7428").decode().unwrap();
        assert_eq!(want, leader_prepare().insert().hash());
        let sig: Signature = Text::new("validator:signature:bn254:065c45ecee3d7363b09f17b3dff610ff33a98d0ea0a90b8dd364fde48660bffc").decode().unwrap();
        sig.verify_hash(&want, &keys()[0].public()).unwrap();
    }
}
