use anyhow::Context as _;
use zksync_consensus_crypto::Text;
use zksync_consensus_utils::enum_util::Variant as _;

use super::{leader_proposal, replica_commit, replica_new_view, replica_timeout};
use crate::validator::{
    self,
    messages::tests::{genesis_v2, validator_keys},
    GenesisHash, Msg, MsgHash,
};

/// Asserts that msg.hash()==hash and that sig is a
/// valid signature of msg (signed by `keys()[0]`).
#[track_caller]
fn msg_change_detector(msg: Msg, hash: &str, sig: &str) {
    let key = validator_keys()[0].clone();

    (|| {
        // Decode hash and signature.
        let hash: MsgHash = Text::new(hash).decode()?;
        let sig: validator::Signature = Text::new(sig).decode()?;

        // Check if msg.hash() is equal to hash.
        if msg.hash() != hash {
            anyhow::bail!("Hash mismatch");
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

/// Note that genesis is NOT versioned by ProtocolVersion.
/// Even if it was, ALL versions of genesis need to be supported FOREVER,
/// unless we introduce dynamic regenesis.
#[test]
fn genesis_hash_change_detector() {
    let want: GenesisHash = Text::new(
        "genesis_hash:keccak256:879ce5d5cf25cc60c605144b1145d7572bf36b309692d0737b712ca7c0e07e1f",
    )
    .decode()
    .unwrap();
    assert_eq!(want, genesis_v2().hash());
}

#[test]
fn replica_commit_change_detector() {
    msg_change_detector(
            replica_commit().insert(),
            "validator_msg:keccak256:71c9220b9e0cbb73fc72aa78c863b556ebf6531b599cc96659dd3d41744c2714",
            "validator:signature:bls12_381:a67c4aa8735d190632e018aa091ac00d8e1f75f71671d9685cbea166482a6d8d8a2be0f011fe7d9c0e1193772a235355",
        );
}

#[test]
fn replica_new_view_change_detector() {
    msg_change_detector(
            replica_new_view().insert(),
            "validator_msg:keccak256:bfdbc016852d4e047f2eaacd66fafc3b6f526a5d5081ffcbeebbd2b84d9d6746",
            "validator:signature:bls12_381:8bf3253c55ed05ffd2c6dd970f7a0d91e5a3c742ce8b7cb58bacefad955369d612f1041391a376e72f6805f4cab08d91",
        );
}

#[test]
fn replica_timeout_change_detector() {
    msg_change_detector(
            replica_timeout().insert(),
            "validator_msg:keccak256:d8318494e9c440af17abedafc0670eda5e57c46da7e8f695a03d9f79b50f1203",
            "validator:signature:bls12_381:8db156653c2bb67ee9d2c6189baab192057d6c3d562811ccb5580cc39407ee91a4a3c1d66dc8e3fda9fd8f6c40d9ac01",
        );
}

#[test]
fn leader_proposal_change_detector() {
    msg_change_detector(
            leader_proposal().insert(),
            "validator_msg:keccak256:9e870d0ba692b68336178f06fa0fe525b093953d7a30810943db25eadf7c05a9",
            "validator:signature:bls12_381:aaab5a11de7bfc3bdc20d80be4f2bf53f1c5f952bf2621876c1574af2c3fb11fc164eeaef9da6eaf9a73e79046053a98",
        );
}
