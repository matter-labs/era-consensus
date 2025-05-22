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
            "validator_msg:keccak256:b2c07fd3ca90233830af980deeaec1dbfa15eb12e43710b13902b29bb81f37da",
            "validator:signature:bls12_381:89bf4a9ff6ab9edf3b16737a77b05b8bd5a04d681bc7ec637d60a45b8fb21e30dcd0a7b7346013ec5acceebebe07326d",
        );
}

#[test]
fn leader_proposal_change_detector() {
    msg_change_detector(
            leader_proposal().insert(),
            "validator_msg:keccak256:d84174e2a5ef6b23dc59c8dc0b101d5a69f8d32d4cdc20b6c18684edf89aa6ed",
            "validator:signature:bls12_381:930444a944b3733ccbd5e345a017a321263b5d6e689611610e1c34642ea521e0f7c459f94ee4dbc9fec6f6635bb4ed16",
        );
}
