use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::test_encode_random;

use super::*;
use crate::validator::{
    self,
    messages::tests::{genesis_v2, payload, validator_keys},
    BlockNumber, GenesisHash, ViewNumber,
};

mod block;
mod consensus;
mod leader_proposal;
mod replica_commit;
mod replica_timeout;
mod version;

/// Hardcoded view
fn view() -> View {
    View {
        genesis: genesis_v2().hash(),
        number: ViewNumber(9136),
        epoch: EpochNumber(7),
    }
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
    let genesis = genesis_v2();
    let replica_commit = replica_commit();
    let mut x = CommitQC::new(
        replica_commit.clone(),
        &genesis.validators_schedule.as_ref().unwrap(),
    );
    for k in validator_keys() {
        x.add(
            &k.sign_msg(replica_commit.clone()),
            genesis.hash(),
            &genesis.validators_schedule.as_ref().unwrap(),
        )
        .unwrap();
    }
    x
}

/// Hardcoded `ReplicaTimeout`
fn replica_timeout() -> ReplicaTimeout {
    ReplicaTimeout {
        view: View {
            genesis: genesis_v2().hash(),
            number: ViewNumber(9169),
            epoch: EpochNumber(8),
        },
        high_vote: Some(replica_commit()),
        high_qc: Some(commit_qc()),
    }
}

/// Hardcoded `TimeoutQC`.
fn timeout_qc() -> TimeoutQC {
    let mut x = TimeoutQC::new(View {
        genesis: genesis_v2().hash(),
        number: ViewNumber(9169),
        epoch: EpochNumber(8),
    });
    let genesis = genesis_v2();
    let replica_timeout = replica_timeout();
    for k in validator_keys() {
        x.add(
            &k.sign_msg(replica_timeout.clone()),
            genesis.hash(),
            &genesis.validators_schedule.as_ref().unwrap(),
        )
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
}
