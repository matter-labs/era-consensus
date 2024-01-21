//! Test-only utilities.
use super::{
    AggregateSignature, BlockHeader, BlockHeaderHash, BlockNumber, CommitQC, ConsensusMsg,
    FinalBlock, LeaderCommit, LeaderPrepare, Msg, MsgHash, NetAddress, Payload, PayloadHash, Phase,
    PrepareQC, ProtocolVersion, PublicKey, ReplicaCommit, ReplicaPrepare, SecretKey, Signature,
    Signed, Signers, ValidatorSet, ViewNumber,
};
use anyhow::{bail, Context};
use bit_vec::BitVec;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;
use zksync_concurrency::time;
use zksync_consensus_utils::enum_util::Variant;

/// Constructs a CommitQC with `CommitQC.message.proposal` matching header.
/// WARNING: it is not a fully correct CommitQC.
pub fn make_justification<R: Rng>(
    rng: &mut R,
    header: &BlockHeader,
    protocol_version: ProtocolVersion,
) -> CommitQC {
    CommitQC {
        message: ReplicaCommit {
            protocol_version,
            view: ViewNumber(header.number.0),
            proposal: *header,
        },
        signers: rng.gen(),
        signature: rng.gen(),
    }
}

impl<'a> BlockBuilder<'a> {
    /// Builds `GenesisSetup`.
    pub fn push(self) {
        let msgs: Vec<_> = self.setup.keys.iter().map(|sk| sk.sign_msg(self.msg.clone())).collect();
        let justification = CommitQC::from(&msgs,&self.setup.validator_set()).unwrap();
        self.setup.blocks.push(FinalBlock { payload: self.payload, justification });
    }

    /// Sets `protocol_version`.
    pub fn protocol_version(mut self, protocol_version: ProtocolVersion) -> Self {
        self.msg.protocol_version = protocol_version;
        self
    }

    /// Sets `block_number`.
    pub fn block_number(mut self, block_number: BlockNumber) -> Self {
        self.msg.proposal.number = block_number;
        self
    }

    /// Sets `payload`.
    pub fn payload(mut self, payload: Payload) -> Self {
        self.msg.proposal.payload = payload.hash();
        self.payload = payload;
        self
    }
}

/// GenesisSetup.
#[derive(Debug, Clone)]
pub struct GenesisSetup {
    /// Validators' secret keys.
    pub keys: Vec<SecretKey>,
    /// Initial blocks.
    pub blocks: Vec<FinalBlock>,
}

/// Builder of GenesisSetup.
pub struct BlockBuilder<'a> {
    setup: &'a mut GenesisSetup,
    msg: ReplicaCommit,
    payload: Payload,
}


impl GenesisSetup {
    /// Constructs GenesisSetup with no blocks.
    pub fn empty(rng: &mut impl Rng, validators: usize) -> Self {
        Self {
            keys: (0..validators).map(|_|rng.gen()).collect(),
            blocks: vec![],
        }
    }

    /// Constructs GenesisSetup with genesis block.
    pub fn new(rng: &mut impl Rng, validators: usize) -> Self {
        let mut this = Self::empty(rng,validators);
        this.push_block(rng.gen());
        this
    }

    /// Returns a builder for the next block.
    pub fn next_block(&mut self) -> BlockBuilder {
        let parent = self.blocks.last().map(|b|b.justification.message.clone());
        let payload = Payload(vec![]);
        BlockBuilder {
            setup: self,
            msg: ReplicaCommit {
                protocol_version: parent.map(|m|m.protocol_version).unwrap_or(ProtocolVersion::EARLIEST),
                view: parent.map(|m|m.view.next()).unwrap_or(ViewNumber(0)),
                proposal: parent
                    .map(|m|BlockHeader::new(&m.proposal,payload.hash()))
                    .unwrap_or(BlockHeader::genesis(payload.hash(),BlockNumber(0))),
            },
            payload,
        }
    }

    /// Pushes the next block with the given payload.
    pub fn push_block(&mut self, payload: Payload) {
        self.next_block().payload(payload).push();
    }

    /// Pushes `count` blocks with a random payload.
    pub fn push_blocks(&mut self, rng: &mut impl Rng, count: usize) {
        for _ in 0..count {
            self.push_block(rng.gen());
        }
    }

    /// ValidatorSet.
    pub fn validator_set(&self) -> ValidatorSet {
        ValidatorSet::new(self.keys.iter().map(|k|k.public())).unwrap()
    }
}


/// Constructs a genesis block with random payload.
pub fn make_genesis_block(rng: &mut impl Rng, protocol_version: ProtocolVersion) -> FinalBlock {
    let mut setup = GenesisSetup::new(rng,3);
    setup.next_block()
        .protocol_version(protocol_version)
        .payload(rng.gen())
        .push();
    setup.blocks[0].clone()
}

/// Constructs a random block with a given parent.
/// WARNING: this is not a fully correct FinalBlock.
pub fn make_block<R: Rng>(
    rng: &mut R,
    parent: &BlockHeader,
    protocol_version: ProtocolVersion,
) -> FinalBlock {
    let payload: Payload = rng.gen();
    let header = BlockHeader::new(parent, payload.hash());
    let justification = make_justification(rng, &header, protocol_version);
    FinalBlock {
        payload,
        justification,
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

impl PrepareQC {
    /// Creates a new PrepareQC from a list of *signed* replica Prepare messages and the current validator set.
    pub fn from(
        signed_messages: &[Signed<ReplicaPrepare>],
        validators: &ValidatorSet,
    ) -> anyhow::Result<Self> {
        // Get the view number from the messages, they must all be equal.
        let view = signed_messages.first()
            .context("Empty signed messages vector")?
            .msg
            .view;

        // Create the messages map.
        let mut prepare_qc = PrepareQC::default();

        for signed_message in signed_messages {
            if signed_message.msg.view != view {
                bail!("Signed messages aren't all for the same view.");
            }

            // Get index of the validator in the validator set.
            let index = validators
                .index(&signed_message.key)
                .context("Message signer isn't in the validator set")?;

            prepare_qc.add(signed_message, index, validators);
        }

        Ok(prepare_qc)
    }
}

impl CommitQC {
    /// Creates a new CommitQC from a list of *signed* replica Commit messages and the current validator set.
    /// * `signed_messages` - A list of valid `ReplicaCommit` signed messages. Must contain at least one item.
    /// * `validators` - The validator set.
    pub fn from(
        signed_messages: &[Signed<ReplicaCommit>],
        validators: &ValidatorSet,
    ) -> anyhow::Result<Self> {
        // Store the signed messages in a Hashmap.
        let message = signed_messages[0].msg;
        let mut commit_qc = CommitQC::new(message, validators);

        for signed_message in signed_messages {
            // Check that the votes are all for the same message.
            if signed_message.msg != message {
                bail!("CommitQC can only be created from votes for the same message.");
            }

            // Get index of the validator in the validator set.
            let validator_index = validators
                .index(&signed_message.key)
                .context("Message signer isn't in the validator set")?;

            commit_qc.add(&signed_message.sig, validator_index);
        }

        Ok(commit_qc)
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

impl Distribution<PayloadHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PayloadHash {
        PayloadHash(rng.gen())
    }
}

impl Distribution<BlockHeaderHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BlockHeaderHash {
        BlockHeaderHash(rng.gen())
    }
}

impl Distribution<BlockHeader> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BlockHeader {
        BlockHeader {
            parent: rng.gen(),
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
            protocol_version: rng.gen(),
            view: rng.gen(),
            high_vote: rng.gen(),
            high_qc: rng.gen(),
        }
    }
}

impl Distribution<ReplicaCommit> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaCommit {
        ReplicaCommit {
            protocol_version: rng.gen(),
            view: rng.gen(),
            proposal: rng.gen(),
        }
    }
}

impl Distribution<LeaderPrepare> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> LeaderPrepare {
        LeaderPrepare {
            protocol_version: rng.gen(),
            view: rng.gen(),
            proposal: rng.gen(),
            proposal_payload: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<LeaderCommit> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> LeaderCommit {
        LeaderCommit {
            protocol_version: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<PrepareQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PrepareQC {
        let n = rng.gen_range(1..11);
        let map = (0..n).map(|_| (rng.gen(), rng.gen())).collect();

        PrepareQC {
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

impl Distribution<ValidatorSet> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ValidatorSet {
        let count = rng.gen_range(1..11);
        let public_keys = (0..count).map(|_| rng.gen());
        ValidatorSet::new(public_keys).unwrap()
    }
}

impl Distribution<ViewNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ViewNumber {
        ViewNumber(rng.gen())
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
