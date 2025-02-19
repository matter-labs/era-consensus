use super::*;
use crate::validator::{messages::testonly::*, BlockNumber, Committee, WeightedValidator};

/// Hardcoded view
pub fn view() -> View {
    View {
        genesis: genesis().hash(),
        number: ViewNumber(9136),
    }
}

/// Hardcoded view numbers.
pub fn views() -> impl Iterator<Item = ViewNumber> {
    [2297, 7203, 8394, 9089, 99821].into_iter().map(ViewNumber)
}

/// Hardcoded `BlockHeader`.
pub fn block_header() -> BlockHeader {
    BlockHeader {
        number: BlockNumber(7728),
        payload: payload().hash(),
    }
}

/// Hardcoded validator committee.
pub fn validator_committee() -> Committee {
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

/// Hardcoded `LeaderProposal`.
pub fn leader_proposal() -> LeaderProposal {
    LeaderProposal {
        proposal_payload: Some(payload()),
        justification: ProposalJustification::Timeout(timeout_qc()),
    }
}

/// Hardcoded `ReplicaCommit`.
pub fn replica_commit() -> ReplicaCommit {
    ReplicaCommit {
        view: view(),
        proposal: block_header(),
    }
}

/// Hardcoded `CommitQC`.
pub fn commit_qc() -> CommitQC {
    let genesis = genesis();
    let replica_commit = replica_commit();
    let mut x = CommitQC::new(replica_commit.clone(), &genesis);
    for k in validator_keys() {
        x.add(&k.sign_msg(replica_commit.clone()), &genesis)
            .unwrap();
    }
    x
}

/// Hardcoded `ReplicaTimeout`
pub fn replica_timeout() -> ReplicaTimeout {
    ReplicaTimeout {
        view: View {
            genesis: genesis().hash(),
            number: ViewNumber(9169),
        },
        high_vote: Some(replica_commit()),
        high_qc: Some(commit_qc()),
    }
}

/// Hardcoded `TimeoutQC`.
pub fn timeout_qc() -> TimeoutQC {
    let mut x = TimeoutQC::new(View {
        genesis: genesis().hash(),
        number: ViewNumber(9169),
    });
    let genesis = genesis();
    let replica_timeout = replica_timeout();
    for k in validator_keys() {
        x.add(&k.sign_msg(replica_timeout.clone()), &genesis)
            .unwrap();
    }
    x
}

/// Hardcoded `ReplicaNewView`.
pub fn replica_new_view() -> ReplicaNewView {
    ReplicaNewView {
        justification: ProposalJustification::Commit(commit_qc()),
    }
}
