//! Test-only utilities.
use super::{
    AggregateSignature, BlockHeader, BlockNumber, ChainId, CommitQC, Committee, ConsensusMsg,
    FinalBlock, ForkNumber, Genesis, GenesisHash, GenesisRaw, LeaderCommit, LeaderPrepare, Msg,
    MsgHash, NetAddress, Payload, PayloadHash, Phase, PrepareQC, ProtocolVersion, PublicKey,
    ReplicaCommit, ReplicaPrepare, SecretKey, Signature, Signed, Signers, View, ViewNumber,
    WeightedValidator,
};
use crate::{attester, validator::LeaderSelectionMode};
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
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// Validator secret keys and weights.
    pub validator_weights: Vec<(SecretKey, u64)>,
    /// Attester secret keys and weights.
    pub attester_weights: Vec<(attester::SecretKey, u64)>,
    /// Leader selection.
    pub leader_selection: LeaderSelectionMode,
}

/// Test setup.
#[derive(Debug, Clone)]
pub struct Setup(SetupInner);

impl SetupSpec {
    /// New `SetupSpec`.
    pub fn new(rng: &mut impl Rng, validators: usize) -> Self {
        Self::new_with_weights(rng, vec![1; validators])
    }

    /// New `SetupSpec`.
    pub fn new_with_weights(rng: &mut impl Rng, weights: Vec<u64>) -> Self {
        Self {
            validator_weights: weights
                .clone()
                .into_iter()
                .map(|w| (rng.gen(), w))
                .collect(),
            attester_weights: weights.into_iter().map(|w| (rng.gen(), w)).collect(),
            chain_id: ChainId(1337),
            fork_number: ForkNumber(rng.gen_range(0..100)),
            first_block: BlockNumber(rng.gen_range(0..100)),
            protocol_version: ProtocolVersion::CURRENT,
            leader_selection: LeaderSelectionMode::RoundRobin,
        }
    }
}

impl Setup {
    /// New `Setup`.
    pub fn new(rng: &mut impl Rng, validators: usize) -> Self {
        SetupSpec::new(rng, validators).into()
    }

    /// New `Setup`.
    pub fn new_with_weights(rng: &mut impl Rng, weights: Vec<u64>) -> Self {
        SetupSpec::new_with_weights(rng, weights).into()
    }

    /// Next block to finalize.
    pub fn next(&self) -> BlockNumber {
        match self.0.blocks.last() {
            Some(b) => b.header().number.next(),
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
                .map(|b| b.justification.view().number.next())
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
        self.0.blocks.push(FinalBlock {
            payload,
            justification,
        });
    }

    /// Pushes `count` blocks with a random payload.
    pub fn push_blocks(&mut self, rng: &mut impl Rng, count: usize) {
        for _ in 0..count {
            self.push_block(rng.gen());
        }
    }

    /// Finds the block by the number.
    pub fn block(&self, n: BlockNumber) -> Option<&FinalBlock> {
        let first = self.0.blocks.first()?.number();
        self.0.blocks.get(n.0.checked_sub(first.0)? as usize)
    }

    /// Pushes a new L1 batch.
    pub fn push_batch(&mut self, batch: attester::Batch) {
        for key in &self.0.attester_keys {
            let signed = key.sign_msg(batch.clone());
            self.0.signed_batches.push(signed);
        }
    }
}

impl From<SetupSpec> for Setup {
    fn from(spec: SetupSpec) -> Self {
        Self(SetupInner {
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
                attesters: attester::Committee::new(spec.attester_weights.iter().map(|(k, w)| {
                    attester::WeightedAttester {
                        key: k.public(),
                        weight: *w,
                    }
                }))
                .unwrap()
                .into(),
                leader_selection: spec.leader_selection,
            }
            .with_hash(),
            validator_keys: spec.validator_weights.into_iter().map(|(k, _)| k).collect(),
            attester_keys: spec.attester_weights.into_iter().map(|(k, _)| k).collect(),
            signed_batches: vec![],
            blocks: vec![],
        })
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
    pub blocks: Vec<FinalBlock>,
    /// L1 batches
    pub signed_batches: Vec<attester::Signed<attester::Batch>>,
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
        GenesisRaw {
            chain_id: rng.gen(),
            fork_number: rng.gen(),
            first_block: rng.gen(),

            protocol_version: rng.gen(),
            validators: rng.gen(),
            attesters: rng.gen(),
            leader_selection: rng.gen(),
        }
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

impl Distribution<ReplicaPrepare> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaPrepare {
        ReplicaPrepare {
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

impl Distribution<LeaderPrepare> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> LeaderPrepare {
        LeaderPrepare {
            proposal: rng.gen(),
            proposal_payload: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<LeaderCommit> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> LeaderCommit {
        LeaderCommit {
            justification: rng.gen(),
        }
    }
}

impl Distribution<PrepareQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PrepareQC {
        let n = rng.gen_range(1..11);
        let map = (0..n).map(|_| (rng.gen(), rng.gen())).collect();

        PrepareQC {
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
            0 => Msg::Consensus(rng.gen()),
            1 => Msg::SessionId(rng.gen()),
            2 => Msg::NetAddress(rng.gen()),
            _ => unreachable!(),
        }
    }
}

impl Distribution<ConsensusMsg> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ConsensusMsg {
        match rng.gen_range(0..4) {
            0 => ConsensusMsg::ReplicaPrepare(rng.gen()),
            1 => ConsensusMsg::ReplicaCommit(rng.gen()),
            2 => ConsensusMsg::LeaderPrepare(rng.gen()),
            3 => ConsensusMsg::LeaderCommit(rng.gen()),
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
