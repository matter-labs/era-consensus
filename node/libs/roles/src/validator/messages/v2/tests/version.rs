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
        "genesis_hash:keccak256:cd8197995830cd694ba99d4c56910c6b89a3bf9af8190d462a32eead9675063e",
    )
    .decode()
    .unwrap();
    assert_eq!(want, genesis_v2().hash());
}

#[test]
fn replica_commit_change_detector() {
    msg_change_detector(
            replica_commit().insert(),
            "validator_msg:keccak256:2c8e94251621366d4502870b24d9790077d87111f355eee34fde1738170b3198",
            "validator:signature:bls12_381:8e89699b1499927d4686034dd9d0045348030303acf48a2e643c825a062aa16ed97ae367087571eff00af4b442f8884f",
        );
}

#[test]
fn replica_new_view_change_detector() {
    msg_change_detector(
            replica_new_view().insert(),
            "validator_msg:keccak256:f672a90ada640d8e0b7aaa54922279e926623964fbabceb462ab743670b6f066",
            "validator:signature:bls12_381:8a2e4950aed464347a3f40148a501a83cbce53ababa9e2d0792388e2b4f89431590df8bd43b63e1d684f70b3122e021d",
        );
}

#[test]
fn replica_timeout_change_detector() {
    msg_change_detector(
            replica_timeout().insert(),
            "validator_msg:keccak256:460e9bb9bce674fec1982a26306ea54b04cb23ea14b68dedcdfc4456e9321c66",
            "validator:signature:bls12_381:afb81f25ee659bac9e61d6ad2ce360a5460763bfdf2e327f0a8585f000e717d9a3eb5b2cc1459d0549ebe5df4fb195d9",
        );
}

#[test]
fn leader_proposal_change_detector() {
    msg_change_detector(
            leader_proposal().insert(),
            "validator_msg:keccak256:0f5fd2b98a46a183ff18fda648e71eeeef897a02adf2a7ce95835367fa44c79a",
            "validator:signature:bls12_381:a79777125067c0b00aa31ec0486a43f904919ae723496c13412350632c141882bea864bf3cceb5acefc217f3c79451c5",
        );
}
