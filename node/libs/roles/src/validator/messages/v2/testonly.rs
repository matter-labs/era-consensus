use bit_vec::BitVec;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};

use super::{
    BlockHeader, ChonkyV2State, CommitQC, FinalBlock, LeaderProposal, Phase, ProposalJustification,
    ReplicaCommit, ReplicaNewView, ReplicaTimeout, Signers, TimeoutQC, View,
};
use crate::validator::{testonly::Setup, Block, EpochNumber, Payload, ViewNumber};

// Adds v2-specific test utilities to the `Setup` struct.
impl Setup {
    /// Pushes the next block with the given payload.
    pub fn push_block_v2(&mut self, payload: Payload) {
        let view = View {
            genesis: self.genesis.hash(),
            number: self
                .0
                .blocks
                .last()
                .map(|b| match b {
                    Block::FinalV2(b) => b.justification.view().number.next(),
                    Block::FinalV1(b) => ViewNumber(b.justification.view().number.next().0),
                    Block::PreGenesis(_) => ViewNumber(0),
                })
                .unwrap_or(ViewNumber(0)),
            epoch: EpochNumber(0),
        };
        let proposal = match self.0.blocks.last() {
            Some(b) => BlockHeader {
                number: b.number().next(),
                payload: payload.hash(),
            },
            None => BlockHeader {
                number: self.genesis.first_block,
                payload: payload.hash(),
            },
        };
        let msg = ReplicaCommit { view, proposal };
        let mut justification =
            CommitQC::new(msg, self.0.genesis.validators_schedule.as_ref().unwrap());
        for key in &self.0.validator_keys {
            justification
                .add(
                    &key.sign_msg(justification.message.clone()),
                    self.0.genesis.hash(),
                    self.0.genesis.validators_schedule.as_ref().unwrap(),
                )
                .unwrap();
        }
        self.0.blocks.push(
            FinalBlock {
                payload,
                justification,
            }
            .into(),
        );
    }

    /// Pushes `count` blocks with a random payload.
    pub fn push_blocks_v2(&mut self, rng: &mut impl Rng, count: usize) {
        for _ in 0..count {
            self.push_block_v2(rng.gen());
        }
    }

    /// Creates a View with the given view number.
    pub fn make_view_v2(&self, number: ViewNumber) -> View {
        View {
            genesis: self.genesis.hash(),
            number,
            epoch: EpochNumber(0),
        }
    }

    /// Creates a ReplicaCommit with a random payload.
    pub fn make_replica_commit_v2(&self, rng: &mut impl Rng, view: ViewNumber) -> ReplicaCommit {
        ReplicaCommit {
            view: self.make_view_v2(view),
            proposal: BlockHeader {
                number: self.next(),
                payload: rng.gen(),
            },
        }
    }

    /// Creates a ReplicaCommit with the given payload.
    pub fn make_replica_commit_with_payload_v2(
        &self,
        payload: &Payload,
        view: ViewNumber,
    ) -> ReplicaCommit {
        ReplicaCommit {
            view: self.make_view_v2(view),
            proposal: BlockHeader {
                number: self.next(),
                payload: payload.hash(),
            },
        }
    }

    /// Creates a CommitQC with a random payload.
    pub fn make_commit_qc_v2(&self, rng: &mut impl Rng, view: ViewNumber) -> CommitQC {
        let mut qc = CommitQC::new(
            self.make_replica_commit_v2(rng, view),
            self.genesis.validators_schedule.as_ref().unwrap(),
        );
        for key in &self.validator_keys {
            qc.add(
                &key.sign_msg(qc.message.clone()),
                self.genesis.hash(),
                self.genesis.validators_schedule.as_ref().unwrap(),
            )
            .unwrap();
        }
        qc
    }

    /// Creates a CommitQC with the given payload.
    pub fn make_commit_qc_with_payload_v2(&self, payload: &Payload, view: ViewNumber) -> CommitQC {
        let mut qc = CommitQC::new(
            self.make_replica_commit_with_payload_v2(payload, view),
            self.genesis.validators_schedule.as_ref().unwrap(),
        );
        for key in &self.validator_keys {
            qc.add(
                &key.sign_msg(qc.message.clone()),
                self.genesis.hash(),
                self.genesis.validators_schedule.as_ref().unwrap(),
            )
            .unwrap();
        }
        qc
    }

    /// Creates a ReplicaTimeout with a random payload.
    pub fn make_replica_timeout_v2(&self, rng: &mut impl Rng, view: ViewNumber) -> ReplicaTimeout {
        let high_vote_view = ViewNumber(rng.gen_range(0..=view.0));
        let high_qc_view = ViewNumber(rng.gen_range(0..high_vote_view.0));
        ReplicaTimeout {
            view: self.make_view_v2(view),
            high_vote: Some(self.make_replica_commit_v2(rng, high_vote_view)),
            high_qc: Some(self.make_commit_qc_v2(rng, high_qc_view)),
        }
    }

    /// Creates a TimeoutQC. If a payload is given, the QC will contain a
    /// re-proposal for that payload
    pub fn make_timeout_qc_v2(
        &self,
        rng: &mut impl Rng,
        view: ViewNumber,
        payload_opt: Option<&Payload>,
    ) -> TimeoutQC {
        let mut vote = if let Some(payload) = payload_opt {
            self.make_replica_commit_with_payload_v2(payload, view.prev().unwrap())
        } else {
            self.make_replica_commit_v2(rng, view.prev().unwrap())
        };
        let commit_qc = match self.0.blocks.last().unwrap() {
            Block::FinalV2(block) => block.justification.clone(),
            _ => unreachable!(),
        };

        let mut qc = TimeoutQC::new(self.make_view_v2(view));
        if payload_opt.is_none() {
            vote.proposal.payload = rng.gen();
        }
        let msg = ReplicaTimeout {
            view: self.make_view_v2(view),
            high_vote: Some(vote.clone()),
            high_qc: Some(commit_qc.clone()),
        };
        for key in &self.validator_keys {
            qc.add(
                &key.sign_msg(msg.clone()),
                self.genesis.hash(),
                self.genesis.validators_schedule.as_ref().unwrap(),
            )
            .unwrap();
        }

        qc
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

impl Distribution<EpochNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> EpochNumber {
        EpochNumber(rng.gen())
    }
}

impl Distribution<View> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> View {
        View {
            genesis: rng.gen(),
            number: rng.gen(),
            epoch: rng.gen(),
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

impl Distribution<ChonkyV2State> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ChonkyV2State {
        ChonkyV2State {
            view_number: rng.gen(),
            epoch_number: rng.gen(),
            phase: rng.gen(),
            high_vote: rng.gen(),
            high_commit_qc: rng.gen(),
            high_timeout_qc: rng.gen(),
            proposals: (0..rng.gen_range(1..11)).map(|_| rng.gen()).collect(),
        }
    }
}
