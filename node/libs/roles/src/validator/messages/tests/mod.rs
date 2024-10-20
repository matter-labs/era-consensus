use super::*;
use crate::{
    attester::{self, WeightedAttester},
    validator::{self, testonly::Setup},
};
use rand::Rng;
use zksync_consensus_crypto::Text;

mod block;
mod committee;
mod consensus;
mod genesis;
mod leader_proposal;
mod replica_commit;
mod replica_timeout;
mod versions;

/// Hardcoded view.
fn view() -> View {
    View {
        genesis: genesis_empty_attesters().hash(),
        number: ViewNumber(9136),
    }
}

/// Hardcoded view numbers.
fn views() -> impl Iterator<Item = ViewNumber> {
    [2297, 7203, 8394, 9089, 99821].into_iter().map(ViewNumber)
}

/// Hardcoded payload.
fn payload() -> Payload {
    Payload(
        hex::decode("57b79660558f18d56b5196053f64007030a1cb7eeadb5c32d816b9439f77edf5f6bd9d")
            .unwrap(),
    )
}

/// Hardcoded `BlockHeader`.
fn block_header() -> BlockHeader {
    BlockHeader {
        number: BlockNumber(7728),
        payload: payload().hash(),
    }
}

/// Hardcoded validator secret keys.
fn validator_keys() -> Vec<validator::SecretKey> {
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

/// Hardcoded genesis with no attesters.
fn genesis_empty_attesters() -> Genesis {
    GenesisRaw {
        chain_id: ChainId(1337),
        fork_number: ForkNumber(42),
        first_block: BlockNumber(2834),

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
        fork_number: ForkNumber(42),
        first_block: BlockNumber(2834),

        protocol_version: ProtocolVersion(1),
        validators: validator_committee(),
        attesters: attester_committee().into(),
        leader_selection: LeaderSelectionMode::Weighted,
    }
    .with_hash()
}

/// Hardcoded `LeaderProposal`.
fn leader_proposal() -> LeaderProposal {
    LeaderProposal {
        proposal: block_header(),
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
    let genesis = genesis_empty_attesters();
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
            genesis: genesis_empty_attesters().hash(),
            number: ViewNumber(9169),
        },
        high_vote: Some(replica_commit()),
        high_qc: Some(commit_qc()),
    }
}

/// Hardcoded `TimeoutQC`.
fn timeout_qc() -> TimeoutQC {
    let mut x = TimeoutQC::new(View {
        genesis: genesis_empty_attesters().hash(),
        number: ViewNumber(9169),
    });
    let genesis = genesis_empty_attesters();
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
