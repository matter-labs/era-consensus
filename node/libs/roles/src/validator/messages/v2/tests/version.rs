use anyhow::Context as _;
use zksync_consensus_crypto::Text;
use zksync_consensus_utils::enum_util::Variant as _;

use crate::validator::{
    self,
    messages::tests::{genesis_v2, validator_keys},
    GenesisHash, Msg, MsgHash,
};

use super::{leader_proposal, replica_commit, replica_new_view, replica_timeout};

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
        "genesis_hash:keccak256:e898a78bbd1de62014bdd62c1a311d766304940b8895191c205234a9e8b2296c",
    )
    .decode()
    .unwrap();
    assert_eq!(want, genesis_v2().hash());
}

#[test]
fn replica_commit_change_detector() {
    msg_change_detector(
            replica_commit().insert(),
            "validator_msg:keccak256:9b39803ff46d80badb5908b9e71b530324ad1b81f8af70bd5bdd03009483fe15",
            "validator:signature:bls12_381:ad574083b1f55d4aadfba7978ef22cf9aa172c537b9e2e35659a07380f81a953d3cb90a620c5a6d46bcc4032cba79d1b",
        );
}

#[test]
fn replica_new_view_change_detector() {
    msg_change_detector(
            replica_new_view().insert(),
            "validator_msg:keccak256:c44c802affaa6256f9be5cde0cb359d728da2431e656d7909be65718753c2c0f",
            "validator:signature:bls12_381:990f10e2e783f3d6abfa11bb5d7123a4505034d8b46532740074b3f3aceeed345741df5ba109617fd1edec1f7dfed2cb",
        );
}

#[test]
fn replica_timeout_change_detector() {
    msg_change_detector(
            replica_timeout().insert(),
            "validator_msg:keccak256:402344f544f53c8ae0962a95cb0b60c06b0a02cbe8dc4b686cc4f8627f5266d4",
            "validator:signature:bls12_381:86cbdcb018ec2f75b58f13e40dde3a22db3d642e5dc7263da4656816a63ca8d8d928bb5306bbec5aa4e6e40035d996d8",
        );
}

#[test]
fn leader_proposal_change_detector() {
    msg_change_detector(
            leader_proposal().insert(),
            "validator_msg:keccak256:e183b29247763a535bb50cc8a179f86e8a50acc890feea9a5558154b070f2951",
            "validator:signature:bls12_381:919c6a5f894c41012986dc5164298e8e5e493429d99c52cb51f8853cd65cdc15b25b7d6bd92e055a1b8dcf97467ad41f",
        );
}
