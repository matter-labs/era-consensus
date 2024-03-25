//! Test-only utilities.
use super::{
    AggregateSignature, BlockHeader, BlockNumber, CommitQC, ConsensusMsg, FinalBlock, Fork,
    ForkNumber, Genesis, GenesisHash, LeaderCommit, LeaderPrepare, Msg, MsgHash, NetAddress,
    Payload, PayloadHash, Phase, PrepareQC, ProtocolVersion, PublicKey, ReplicaCommit,
    ReplicaPrepare, SecretKey, Signature, Signed, Signers, ValidatorCommittee, View, ViewNumber,
    WeightedValidator,
};
use bit_vec::BitVec;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;
use zksync_concurrency::time;
use zksync_consensus_utils::enum_util::Variant;

/// Test setup.
#[derive(Debug, Clone)]
pub struct Setup(SetupInner);

impl Setup {
    /// New `Setup` with a given `fork`.
    pub fn new_with_fork(rng: &mut impl Rng, validators: usize, fork: Fork) -> Self {
        let keys: Vec<SecretKey> = (0..validators).map(|_| rng.gen()).collect();
        let weight = ValidatorCommittee::MAX_WEIGHT / validators as u64;
        let genesis = Genesis {
            validators: ValidatorCommittee::new(keys.iter().map(|k| WeightedValidator {
                key: k.public(),
                weight,
            }))
            .unwrap(),
            fork,
        };
        Self(SetupInner {
            keys,
            genesis,
            blocks: vec![],
        })
    }

    /// New `Setup`.
    pub fn new(rng: &mut impl Rng, validators: usize) -> Self {
        let fork = Fork {
            number: ForkNumber(rng.gen_range(0..100)),
            first_block: BlockNumber(rng.gen_range(0..100)),
        };
        Self::new_with_fork(rng, validators, fork)
    }

    /// Next block to finalize.
    pub fn next(&self) -> BlockNumber {
        match self.0.blocks.last() {
            Some(b) => b.header().number.next(),
            None => self.0.genesis.fork.first_block,
        }
    }

    /// Pushes the next block with the given payload.
    pub fn push_block(&mut self, payload: Payload) {
        let view = View {
            protocol_version: ProtocolVersion::EARLIEST,
            fork: self.genesis.fork.number,
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
                number: self.genesis.fork.first_block,
                payload: payload.hash(),
            },
        };
        let msg = ReplicaCommit { view, proposal };
        let mut justification = CommitQC::new(msg, &self.0.genesis);
        for key in &self.0.keys {
            justification.add(
                &key.sign_msg(justification.message.clone()),
                &self.0.genesis,
            );
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
}

/// Setup.
#[derive(Debug, Clone)]
pub struct SetupInner {
    /// Validators' secret keys.
    pub keys: Vec<SecretKey>,
    /// Past blocks.
    pub blocks: Vec<FinalBlock>,
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

impl Distribution<GenesisHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GenesisHash {
        GenesisHash(rng.gen())
    }
}

impl Distribution<Fork> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Fork {
        Fork {
            number: rng.gen(),
            first_block: rng.gen(),
        }
    }
}

impl Distribution<Genesis> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Genesis {
        Genesis {
            validators: rng.gen(),
            fork: rng.gen(),
        }
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

impl Distribution<ValidatorCommittee> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ValidatorCommittee {
        let count = rng.gen_range(1..11);
        let public_keys = (0..count).map(|_| WeightedValidator {
            key: rng.gen(),
            weight: ValidatorCommittee::MAX_WEIGHT,
        });
        ValidatorCommittee::new(public_keys).unwrap()
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
            protocol_version: rng.gen(),
            fork: rng.gen(),
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
