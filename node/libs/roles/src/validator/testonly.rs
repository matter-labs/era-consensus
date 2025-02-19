//! Test-only utilities.
use super::messages::v1::{
    BlockHeader, CommitQC, ConsensusMsg, FinalBlock, LeaderProposal, LeaderSelectionMode, Phase,
    ProposalJustification, ReplicaCommit, ReplicaNewView, ReplicaTimeout, Signers, TimeoutQC, View,
    ViewNumber,
};
use super::{
    AggregateSignature, Block, BlockNumber, ChainId, Committee, ForkNumber, Genesis, GenesisHash,
    GenesisRaw, Justification, Msg, MsgHash, NetAddress, Payload, PayloadHash, PreGenesisBlock,
    ProofOfPossession, ProtocolVersion, PublicKey, SecretKey, Signature, Signed, WeightedValidator,
};
use crate::attester;
use bit_vec::BitVec;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;
use zksync_concurrency::time;
use zksync_consensus_utils::enum_util::Variant;

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

impl AggregateSignature {
    /// Generate a new aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        let mut agg = Self::default();
        for sig in sigs {
            agg.add(sig);
        }
        agg
    }
}

impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        AggregateSignature(rng.gen())
    }
}

impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        PublicKey(rng.gen())
    }
}

impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        Signature(rng.gen())
    }
}

impl Distribution<ProofOfPossession> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ProofOfPossession {
        ProofOfPossession(rng.gen())
    }
}

impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        SecretKey(Arc::new(rng.gen()))
    }
}

impl Distribution<BlockNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BlockNumber {
        BlockNumber(rng.gen())
    }
}

impl Distribution<ProtocolVersion> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ProtocolVersion {
        ProtocolVersion(rng.gen())
    }
}

impl Distribution<ForkNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ForkNumber {
        ForkNumber(rng.gen())
    }
}

impl Distribution<ChainId> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ChainId {
        ChainId(rng.gen())
    }
}

impl Distribution<GenesisHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GenesisHash {
        GenesisHash(rng.gen())
    }
}

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

impl Distribution<GenesisRaw> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GenesisRaw {
        let mut genesis = GenesisRaw {
            chain_id: rng.gen(),
            fork_number: rng.gen(),
            first_block: rng.gen(),

            protocol_version: rng.gen(),
            validators: rng.gen(),
            leader_selection: rng.gen(),
        };

        // In order for the genesis to be valid, sticky/rota leaders need to be in the validator committee.
        if let LeaderSelectionMode::Sticky(_) = genesis.leader_selection {
            let i = rng.gen_range(0..genesis.validators.len());
            genesis.leader_selection =
                LeaderSelectionMode::Sticky(genesis.validators.get(i).unwrap().key.clone());
        } else if let LeaderSelectionMode::Rota(pks) = genesis.leader_selection {
            let n = pks.len();
            let mut pks = Vec::new();
            for _ in 0..n {
                let i = rng.gen_range(0..genesis.validators.len());
                pks.push(genesis.validators.get(i).unwrap().key.clone());
            }
            genesis.leader_selection = LeaderSelectionMode::Rota(pks);
        }

        genesis
    }
}

impl Distribution<Genesis> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Genesis {
        rng.gen::<GenesisRaw>().with_hash()
    }
}

impl Distribution<PayloadHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PayloadHash {
        PayloadHash(rng.gen())
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

impl Distribution<Payload> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Payload {
        let size: usize = rng.gen_range(500..1000);
        Payload((0..size).map(|_| rng.gen()).collect())
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

impl Distribution<Justification> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Justification {
        let size: usize = rng.gen_range(500..1000);
        Justification((0..size).map(|_| rng.gen()).collect())
    }
}

impl Distribution<PreGenesisBlock> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PreGenesisBlock {
        PreGenesisBlock {
            number: rng.gen(),
            payload: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<Block> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Block {
        match rng.gen_range(0..2) {
            0 => Block::PreGenesis(rng.gen()),
            _ => Block::FinalV1(rng.gen()),
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

impl Distribution<Committee> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Committee {
        let count = rng.gen_range(1..11);
        let public_keys = (0..count).map(|_| WeightedValidator {
            key: rng.gen(),
            weight: 1,
        });
        Committee::new(public_keys).unwrap()
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

impl Distribution<NetAddress> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> NetAddress {
        NetAddress {
            addr: std::net::SocketAddr::new(
                std::net::IpAddr::from(rng.gen::<[u8; 16]>()),
                rng.gen(),
            ),
            version: rng.gen(),
            timestamp: time::UNIX_EPOCH + time::Duration::seconds(rng.gen_range(0..1000000000)),
        }
    }
}

impl Distribution<Msg> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Msg {
        match rng.gen_range(0..3) {
            0 => Msg::ConsensusV1(rng.gen()),
            1 => Msg::SessionId(rng.gen()),
            2 => Msg::NetAddress(rng.gen()),
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

impl Distribution<MsgHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MsgHash {
        MsgHash(rng.gen())
    }
}

impl<V: Variant<Msg>> Distribution<Signed<V>> for Standard
where
    Standard: Distribution<V>,
{
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signed<V> {
        rng.gen::<SecretKey>().sign_msg(rng.gen())
    }
}
