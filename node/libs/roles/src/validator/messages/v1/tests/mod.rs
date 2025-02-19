use super::*;
use crate::validator::{
    self,
    messages::tests::{genesis_v1, payload, validator_keys},
    BlockNumber, GenesisHash, Msg, MsgHash,
};
use anyhow::Context as _;
use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::Variant as _;
use zksync_protobuf::testonly::test_encode_random;

mod block;
mod consensus;
mod leader_proposal;
mod replica_commit;
mod replica_timeout;
mod version;

/// Hardcoded view
fn view() -> View {
    View {
        genesis: genesis_v1().hash(),
        number: ViewNumber(9136),
    }
}

/// Hardcoded view numbers.
fn views() -> impl Iterator<Item = ViewNumber> {
    [2297, 7203, 8394, 9089, 99821].into_iter().map(ViewNumber)
}

/// Hardcoded `BlockHeader`.
fn block_header() -> BlockHeader {
    BlockHeader {
        number: BlockNumber(7728),
        payload: payload().hash(),
    }
}

/// Hardcoded `LeaderProposal`.
fn leader_proposal() -> LeaderProposal {
    LeaderProposal {
        proposal_payload: Some(payload()),
        justification: ProposalJustification::Timeout(timeout_qc()),
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
    for k in validator_keys() {
        x.add(&k.sign_msg(replica_commit.clone()), &genesis)
            .unwrap();
    }
    x
}

/// Hardcoded `ReplicaTimeout`
fn replica_timeout() -> ReplicaTimeout {
    ReplicaTimeout {
        view: View {
            genesis: genesis_v1().hash(),
            number: ViewNumber(9169),
        },
        high_vote: Some(replica_commit()),
        high_qc: Some(commit_qc()),
    }
}

/// Hardcoded `TimeoutQC`.
fn timeout_qc() -> TimeoutQC {
    let mut x = TimeoutQC::new(View {
        genesis: genesis_v1().hash(),
        number: ViewNumber(9169),
    });
    let genesis = genesis_v1();
    let replica_timeout = replica_timeout();
    for k in validator_keys() {
        x.add(&k.sign_msg(replica_timeout.clone()), &genesis)
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

/// Note that genesis is NOT versioned by ProtocolVersion.
/// Even if it was, ALL versions of genesis need to be supported FOREVER,
/// unless we introduce dynamic regenesis.
#[test]
fn genesis_hash_change_detector() {
    let want: GenesisHash = Text::new(
        "genesis_hash:keccak256:75cfa582fcda9b5da37af8fb63a279f777bb17a97a50519e1a61aad6c77a522f",
    )
    .decode()
    .unwrap();
    assert_eq!(want, genesis_v1().hash());
}

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

#[test]
fn replica_commit_change_detector() {
    msg_change_detector(
            replica_commit().insert(),
            "validator_msg:keccak256:ccbb11a6b3f4e06840a2a06abc2a245a2b3de30bb951e759a9ec6920f74f0632",
            "validator:signature:bls12_381:8e41b89c89c0de8f83102966596ab95f6bdfdc18fceaceb224753b3ff495e02d5479c709829bd6d0802c5a1f24fa96b5",
        );
}

#[test]
fn replica_new_view_change_detector() {
    msg_change_detector(
            replica_new_view().insert(),
            "validator_msg:keccak256:2be143114cd3442b96d5f6083713c4c338a1c18ef562ede4721ebf037689a6ad",
            "validator:signature:bls12_381:9809b66d44509cf7847baaa03a35ae87062f9827cf1f90c8353f057eee45b79fde0f4c4c500980b69c59263b51b6d072",
        );
}

#[test]
fn replica_timeout_change_detector() {
    msg_change_detector(
            replica_timeout().insert(),
            "validator_msg:keccak256:615fa6d2960b48e30ab88fe195bbad161b8a6f9a59a45ca86b5e2f20593f76cd",
            "validator:signature:bls12_381:ac9b6d340bf1b04421455676b8a28a8de079cd9b40f75f1009aa3da32981690bc520d4ec0284ae030fc8b036d86ca307",
        );
}

#[test]
fn leader_proposal_change_detector() {
    msg_change_detector(
            leader_proposal().insert(),
            "validator_msg:keccak256:4c1b2cf1e8fbb00cde86caee200491df15c45d5c88402e227c1f3e1b416c4255",
            "validator:signature:bls12_381:81f865807067c6f70f17f9716e6d41c0103c2366abb6721408fb7d27ead6332798bd7b34d5f4a63e324082586b2c69a3",
        );
}

#[test]
fn test_byte_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let final_block: FinalBlock = rng.gen();
    assert_eq!(
        final_block,
        ByteFmt::decode(&ByteFmt::encode(&final_block)).unwrap()
    );
}

#[test]
fn test_text_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let genesis_hash: GenesisHash = rng.gen();
    let t = TextFmt::encode(&genesis_hash);
    assert_eq!(genesis_hash, Text::new(&t).decode::<GenesisHash>().unwrap());
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<BlockHeader>(rng);
    test_encode_random::<FinalBlock>(rng);
    test_encode_random::<TimeoutQC>(rng);
    test_encode_random::<CommitQC>(rng);
    test_encode_random::<Signers>(rng);
    test_encode_random::<LeaderSelectionMode>(rng);
}
