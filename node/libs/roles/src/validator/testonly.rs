//! Test-only utilities.
use super::v1::{
    BlockHeader, CommitQC, FinalBlock, LeaderSelectionMode, ReplicaCommit, ReplicaTimeout,
    TimeoutQC, View, ViewNumber,
};
use super::{
    Block, BlockNumber, ChainId, Committee, ForkNumber, Genesis, GenesisRaw, Payload,
    PreGenesisBlock, ProtocolVersion, SecretKey, WeightedValidator,
};
use crate::attester;
use rand::Rng;

/// Test setup specification.
#[derive(Debug, Clone)]
pub struct SetupSpec {
    /// ChainId
    pub chain_id: ChainId,
    /// Fork number.
    pub fork_number: ForkNumber,
    /// First block.
    pub first_block: BlockNumber,
    /// First block that exists.
    pub first_pregenesis_block: BlockNumber,
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// Validator secret keys and weights.
    pub validator_weights: Vec<(SecretKey, u64)>,
    /// Attester secret keys and weights.
    pub attester_weights: Vec<(attester::SecretKey, u64)>,
    /// Leader selection.
    pub leader_selection: LeaderSelectionMode,
}

impl SetupSpec {
    /// New `SetupSpec`.
    pub fn new(rng: &mut impl Rng, validators: usize) -> Self {
        Self::new_with_weights(rng, vec![1; validators])
    }

    /// New `SetupSpec`.
    pub fn new_with_weights(rng: &mut impl Rng, weights: Vec<u64>) -> Self {
        let first_block = BlockNumber(rng.gen_range(0..100));
        Self {
            validator_weights: weights
                .clone()
                .into_iter()
                .map(|w| (rng.gen(), w))
                .collect(),
            attester_weights: weights.into_iter().map(|w| (rng.gen(), w)).collect(),
            chain_id: ChainId(1337),
            fork_number: ForkNumber(rng.gen_range(0..100)),
            first_block,
            first_pregenesis_block: BlockNumber(rng.gen_range(0..=first_block.0)),
            protocol_version: ProtocolVersion::CURRENT,
            leader_selection: LeaderSelectionMode::RoundRobin,
        }
    }
}

/// Setup.
#[derive(Debug, Clone)]
pub struct SetupInner {
    /// Validators' secret keys.
    pub validator_keys: Vec<SecretKey>,
    /// Attesters' secret keys.
    pub attester_keys: Vec<attester::SecretKey>,
    /// Past blocks.
    pub blocks: Vec<Block>,
    /// Genesis config.
    pub genesis: Genesis,
}

impl std::ops::Deref for Setup {
    type Target = SetupInner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Test setup.
#[derive(Debug, Clone)]
pub struct Setup(SetupInner);

impl Setup {
    /// New `Setup`.
    pub fn new(rng: &mut impl Rng, validators: usize) -> Self {
        let spec = SetupSpec::new(rng, validators);
        Self::from_spec(rng, spec)
    }

    /// New `Setup`.
    pub fn new_with_weights(rng: &mut impl Rng, weights: Vec<u64>) -> Self {
        let spec = SetupSpec::new_with_weights(rng, weights);
        Self::from_spec(rng, spec)
    }

    /// Generates a new `Setup` from the given `SetupSpec`.
    pub fn from_spec(rng: &mut impl Rng, spec: SetupSpec) -> Self {
        let mut this = Self(SetupInner {
            genesis: GenesisRaw {
                chain_id: spec.chain_id,
                fork_number: spec.fork_number,
                first_block: spec.first_block,

                protocol_version: spec.protocol_version,
                validators: Committee::new(spec.validator_weights.iter().map(|(k, w)| {
                    WeightedValidator {
                        key: k.public(),
                        weight: *w,
                    }
                }))
                .unwrap(),
                leader_selection: spec.leader_selection,
            }
            .with_hash(),
            validator_keys: spec.validator_weights.into_iter().map(|(k, _)| k).collect(),
            attester_keys: spec.attester_weights.into_iter().map(|(k, _)| k).collect(),
            blocks: vec![],
        });
        // Populate pregenesis blocks.
        for block in spec.first_pregenesis_block.0..spec.first_block.0 {
            this.0.blocks.push(
                PreGenesisBlock {
                    number: BlockNumber(block),
                    payload: rng.gen(),
                    justification: rng.gen(),
                }
                .into(),
            );
        }
        this
    }

    /// Next block to finalize.
    pub fn next(&self) -> BlockNumber {
        match self.0.blocks.last() {
            Some(b) => b.number().next(),
            None => self.0.genesis.first_block,
        }
    }

    /// Pushes the next block with the given payload.
    pub fn push_block(&mut self, payload: Payload) {
        let view = View {
            genesis: self.genesis.hash(),
            number: self
                .0
                .blocks
                .last()
                .map(|b| match b {
                    Block::FinalV1(b) => b.justification.view().number.next(),
                    Block::PreGenesis(_) => ViewNumber(0),
                })
                .unwrap_or(ViewNumber(0)),
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
        let mut justification = CommitQC::new(msg, &self.0.genesis);
        for key in &self.0.validator_keys {
            justification
                .add(
                    &key.sign_msg(justification.message.clone()),
                    &self.0.genesis,
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
    pub fn push_blocks(&mut self, rng: &mut impl Rng, count: usize) {
        for _ in 0..count {
            self.push_block(rng.gen());
        }
    }

    /// Finds the block by the number.
    pub fn block(&self, n: BlockNumber) -> Option<&Block> {
        let first = self.0.blocks.first()?.number();
        self.0.blocks.get(n.0.checked_sub(first.0)? as usize)
    }

    /// Creates a View with the given view number.
    pub fn make_view(&self, number: ViewNumber) -> View {
        View {
            genesis: self.genesis.hash(),
            number,
        }
    }

    /// Creates a ReplicaCommit with a random payload.
    pub fn make_replica_commit(&self, rng: &mut impl Rng, view: ViewNumber) -> ReplicaCommit {
        ReplicaCommit {
            view: self.make_view(view),
            proposal: BlockHeader {
                number: self.next(),
                payload: rng.gen(),
            },
        }
    }

    /// Creates a ReplicaCommit with the given payload.
    pub fn make_replica_commit_with_payload(
        &self,
        payload: &Payload,
        view: ViewNumber,
    ) -> ReplicaCommit {
        ReplicaCommit {
            view: self.make_view(view),
            proposal: BlockHeader {
                number: self.next(),
                payload: payload.hash(),
            },
        }
    }

    /// Creates a CommitQC with a random payload.
    pub fn make_commit_qc(&self, rng: &mut impl Rng, view: ViewNumber) -> CommitQC {
        let mut qc = CommitQC::new(self.make_replica_commit(rng, view), &self.genesis);
        for key in &self.validator_keys {
            qc.add(&key.sign_msg(qc.message.clone()), &self.genesis)
                .unwrap();
        }
        qc
    }

    /// Creates a CommitQC with the given payload.
    pub fn make_commit_qc_with_payload(&self, payload: &Payload, view: ViewNumber) -> CommitQC {
        let mut qc = CommitQC::new(
            self.make_replica_commit_with_payload(payload, view),
            &self.genesis,
        );
        for key in &self.validator_keys {
            qc.add(&key.sign_msg(qc.message.clone()), &self.genesis)
                .unwrap();
        }
        qc
    }

    /// Creates a ReplicaTimeout with a random payload.
    pub fn make_replica_timeout(&self, rng: &mut impl Rng, view: ViewNumber) -> ReplicaTimeout {
        let high_vote_view = ViewNumber(rng.gen_range(0..=view.0));
        let high_qc_view = ViewNumber(rng.gen_range(0..high_vote_view.0));
        ReplicaTimeout {
            view: self.make_view(view),
            high_vote: Some(self.make_replica_commit(rng, high_vote_view)),
            high_qc: Some(self.make_commit_qc(rng, high_qc_view)),
        }
    }

    /// Creates a TimeoutQC. If a payload is given, the QC will contain a
    /// re-proposal for that payload
    pub fn make_timeout_qc(
        &self,
        rng: &mut impl Rng,
        view: ViewNumber,
        payload_opt: Option<&Payload>,
    ) -> TimeoutQC {
        let mut vote = if let Some(payload) = payload_opt {
            self.make_replica_commit_with_payload(payload, view.prev().unwrap())
        } else {
            self.make_replica_commit(rng, view.prev().unwrap())
        };
        let commit_qc = match self.0.blocks.last().unwrap() {
            Block::FinalV1(block) => block.justification.clone(),
            _ => unreachable!(),
        };

        let mut qc = TimeoutQC::new(self.make_view(view));
        if payload_opt.is_none() {
            vote.proposal.payload = rng.gen();
        }
        let msg = ReplicaTimeout {
            view: self.make_view(view),
            high_vote: Some(vote.clone()),
            high_qc: Some(commit_qc.clone()),
        };
        for key in &self.validator_keys {
            qc.add(&key.sign_msg(msg.clone()), &self.genesis).unwrap();
        }

        qc
    }
}
