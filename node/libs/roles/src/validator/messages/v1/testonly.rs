use super::{
    BlockHeader, CommitQC, ConsensusMsg, FinalBlock, LeaderProposal, LeaderSelectionMode, Phase,
    ProposalJustification, ReplicaCommit, ReplicaNewView, ReplicaTimeout, Signers, TimeoutQC, View,
    ViewNumber,
};
use bit_vec::BitVec;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};

impl Distribution<LeaderSelectionMode> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> LeaderSelectionMode {
        match rng.gen_range(0..=3) {
            0 => LeaderSelectionMode::RoundRobin,
            1 => LeaderSelectionMode::Sticky(rng.gen()),
            3 => LeaderSelectionMode::Rota({
                let n = rng.gen_range(1..=3);
                rng.sample_iter(Standard).take(n).collect()
            }),
            _ => LeaderSelectionMode::Weighted,
        }
    }
}

impl Distribution<BlockHeader> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BlockHeader {
        BlockHeader {
            number: rng.gen(),
            payload: rng.gen(),
        }
    }
}

impl Distribution<FinalBlock> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> FinalBlock {
        FinalBlock {
            payload: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<ReplicaTimeout> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaTimeout {
        ReplicaTimeout {
            view: rng.gen(),
            high_vote: rng.gen(),
            high_qc: rng.gen(),
        }
    }
}

impl Distribution<ReplicaCommit> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaCommit {
        ReplicaCommit {
            view: rng.gen(),
            proposal: rng.gen(),
        }
    }
}

impl Distribution<ReplicaNewView> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaNewView {
        ReplicaNewView {
            justification: rng.gen(),
        }
    }
}

impl Distribution<LeaderProposal> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> LeaderProposal {
        LeaderProposal {
            proposal_payload: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<TimeoutQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> TimeoutQC {
        let n = rng.gen_range(1..11);
        let map = (0..n).map(|_| (rng.gen(), rng.gen())).collect();

        TimeoutQC {
            view: rng.gen(),
            map,
            signature: rng.gen(),
        }
    }
}

impl Distribution<CommitQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> CommitQC {
        CommitQC {
            message: rng.gen(),
            signers: rng.gen(),
            signature: rng.gen(),
        }
    }
}

impl Distribution<ProposalJustification> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ProposalJustification {
        match rng.gen_range(0..2) {
            0 => ProposalJustification::Commit(rng.gen()),
            1 => ProposalJustification::Timeout(rng.gen()),
            _ => unreachable!(),
        }
    }
}

impl Distribution<Signers> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signers {
        Signers(BitVec::from_bytes(&rng.gen::<[u8; 4]>()))
    }
}

impl Distribution<ViewNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ViewNumber {
        ViewNumber(rng.gen())
    }
}

impl Distribution<View> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> View {
        View {
            genesis: rng.gen(),
            number: rng.gen(),
        }
    }
}

impl Distribution<Phase> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Phase {
        let i = rng.gen_range(0..2);

        match i {
            0 => Phase::Prepare,
            1 => Phase::Commit,
            _ => unreachable!(),
        }
    }
}

impl Distribution<ConsensusMsg> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ConsensusMsg {
        match rng.gen_range(0..4) {
            0 => ConsensusMsg::LeaderProposal(rng.gen()),
            1 => ConsensusMsg::ReplicaCommit(rng.gen()),
            2 => ConsensusMsg::ReplicaNewView(rng.gen()),
            3 => ConsensusMsg::ReplicaTimeout(rng.gen()),
            _ => unreachable!(),
        }
    }
}
