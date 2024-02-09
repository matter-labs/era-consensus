//! Messages related to the consensus protocol.
use super::{BlockNumber, BlockHeader, Msg, Payload, Signed};
use crate::{validator};
use anyhow::Context as _;
use bit_vec::BitVec;
use std::collections::{BTreeMap, BTreeSet};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};
use zksync_consensus_crypto::{TextFmt, ByteFmt, Text, keccak256::Keccak256};
use std::fmt;

/// Version of the consensus algorithm that the validator is using.
/// It allows to prevent misinterpretation of messages signed by validators
/// using different versions of the binaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub u32);

impl ProtocolVersion {
    /// Earliest protocol version.
    pub const EARLIEST: Self = Self(0);

    /// Returns the integer corresponding to this version.
    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Checks protocol version compatibility.
    pub fn compatible(&self, other: &ProtocolVersion) -> bool {
        // Currently using comparison.
        // This can be changed later to apply a minimum supported version.
        self.0 == other.0
    }
}

impl TryFrom<u32> for ProtocolVersion {
    type Error = anyhow::Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        // Currently, consensus doesn't define restrictions on the possible version. Unsupported
        // versions are filtered out on the BFT actor level instead.
        Ok(Self(value))
    }
}

/// Identifier of the fork that this consensus instance is building blocks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForkId(pub usize);

#[derive(Clone,Debug, PartialEq)]
pub(crate) struct Fork {
    pub(crate) first_block: BlockNumber,
    parent_fork: Option<ForkId>,
}

impl Default for Fork {
    fn default() -> Self { Self { first_block: BlockNumber(0), parent_fork: None } }
}

/// History of forks of the blockchain.
/// It allows to determine which which fork contains block with the given number.
/// Current fork is the one with the highest fork id.
#[derive(Debug,Clone,PartialEq)]
pub struct ForkSet(pub(in crate::validator) Vec<Fork>);

impl Default for ForkSet {
    fn default() -> Self { Self(vec![Fork::default()]) }
}

impl ForkSet {
    /// Inserts a new fork to the fork set, attached to the highest fork with
    /// the block `first_block-1`.
    pub fn insert(&mut self, first_block: BlockNumber) {
        self.0.push(Fork {
            first_block,
            parent_fork: first_block.prev().map(|b|self.find(b)),
        });
    }

    /// Current fork that node is participating in. 
    pub fn current(&self) -> ForkId {
        ForkId(self.0.len()-1)
    }

    /// Finds the fork which the given block belongs to.
    /// This should be used to verify the quorum certificate
    /// on a finalized block fetched from the network.
    pub fn find(&self, block: BlockNumber) -> ForkId {
        let mut i = self.current();
        loop {
            if self.0[i.0].first_block <= block {
                return i;
            }
            // By construction, every block belongs to some fork.
            i = self.0[i.0].parent_fork.unwrap();
        }
    }
}

/// Genesis of the blockchain, unique for each blockchain instance.
#[derive(Debug,Clone, PartialEq)]
pub struct Genesis {
    // TODO(gprusak): add blockchain id here.
    /// Set of validators of the chain.
    pub validators: ValidatorSet,
    /// Forks history, `forks.current()` indicates the current fork.
    pub forks: ForkSet,
}

/// Hash of the genesis specification.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GenesisHash(pub(crate) Keccak256);

impl Genesis {
    /// Hash of the genesis.
    pub fn hash(&self) -> GenesisHash {
        GenesisHash(Keccak256::new(&zksync_protobuf::canonical(self)))
    }
}

impl TextFmt for GenesisHash {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("genesis_hash:keccak256:")?
            .decode_hex()
            .map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "genesis_hash:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl fmt::Debug for GenesisHash {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// Consensus messages.
#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsensusMsg {
    ReplicaPrepare(ReplicaPrepare),
    ReplicaCommit(ReplicaCommit),
    LeaderPrepare(LeaderPrepare),
    LeaderCommit(LeaderCommit),
}

impl ConsensusMsg {
    /// ConsensusMsg variant name.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ReplicaPrepare(_) => "ReplicaPrepare",
            Self::ReplicaCommit(_) => "ReplicaCommit",
            Self::LeaderPrepare(_) => "LeaderPrepare",
            Self::LeaderCommit(_) => "LeaderCommit",
        }
    }

    /// View of this message.
    pub fn view(&self) -> &View {
        match self {
            Self::ReplicaPrepare(m) => &m.view,
            Self::ReplicaCommit(m) => &m.view,
            Self::LeaderPrepare(m) => &m.justification.view,
            Self::LeaderCommit(m) => &m.justification.message.view,
        } 
    }

    /// Protocol version of this message.
    pub fn protocol_version(&self) -> ProtocolVersion {
        self.view().protocol_version
    }
}

impl Variant<Msg> for ReplicaPrepare {
    fn insert(self) -> Msg {
        ConsensusMsg::ReplicaPrepare(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::ReplicaPrepare(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for ReplicaCommit {
    fn insert(self) -> Msg {
        ConsensusMsg::ReplicaCommit(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::ReplicaCommit(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for LeaderPrepare {
    fn insert(self) -> Msg {
        ConsensusMsg::LeaderPrepare(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::LeaderPrepare(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for LeaderCommit {
    fn insert(self) -> Msg {
        ConsensusMsg::LeaderCommit(self).insert()
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let ConsensusMsg::LeaderCommit(this) = Variant::extract(msg)? else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

/// View specification.
#[derive(Clone,Debug,PartialEq,Eq, PartialOrd, Ord, Hash)]
pub struct View {
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// Fork this message belongs to.
    pub fork: ForkId,
    /// The number of the current view.
    pub number: ViewNumber,
}

impl View {
    /// Checks if `self` can occur after `b`.
    pub fn after(&self, b: &Self) -> bool {
        self.fork == b.fork &&
        self.number > b.number &&
        self.protocol_version >= b.protocol_version
    }
}

/// A Prepare message from a replica.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReplicaPrepare {
    /// View of this message.
    pub view: View,
    /// The highest block that the replica has committed to.
    pub high_vote: Option<ReplicaCommit>,
    /// The highest CommitQC that the replica has seen.
    pub high_qc: Option<CommitQC>,
}

impl ReplicaPrepare {
    /// Verifies the message.
    pub fn verify(&self, genesis: &Genesis) -> anyhow::Result<()> {
        if let Some(v) = &self.high_vote {
            anyhow::ensure!(self.view.after(&v.view));
            v.verify(genesis).context("high_vote")?;
        }
        if let Some(qc) = &self.high_qc {
            anyhow::ensure!(self.view.after(&qc.view()));
            qc.verify(genesis).context("high_qc")?;
        }
        Ok(())
    }
}

/// A Commit message from a replica.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicaCommit {
    /// View of this message.
    pub view: View,
    /// The header of the block that the replica is committing to.
    pub proposal: BlockHeader,
}

impl ReplicaCommit {
    /// Verifies the message.
    pub fn verify(&self, _genesis: &Genesis) -> anyhow::Result<()> {
        Ok(())
    }
}

/// A Prepare message from a leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderPrepare { 
    /// The header of the block that the leader is proposing.
    pub proposal: BlockHeader,
    /// Payload of the block that the leader is proposing.
    /// `None` iff this is a reproposal.
    pub proposal_payload: Option<Payload>,
    /// The PrepareQC that justifies this proposal from the leader.
    pub justification: PrepareQC,
}

impl LeaderPrepare {
    /// Verifies LeaderPrepare.
    pub fn verify(&self, genesis: &Genesis) -> anyhow::Result<()> {
        self.justification.verify(genesis).context("justification")?;
        
        // Get the highest block voted and check if there's a quorum of votes for it. To have a quorum
        // in this situation, we require 2*f+1 votes, where f is the maximum number of faulty replicas.
        let mut vote_count: HashMap<_, usize> = HashMap::new();

        for (msg, signers) in &message.justification.map {
            *vote_count.entry(msg.high_vote.proposal.clone()).or_default() += signers.len();
        }
        let faulty_replicas = genesis.validators.faulty_replicas();
        let highest_vote: Option<validator::BlockHeader> = vote_count
            .into_iter()
            // We only take one value from the iterator because there can only be at most one block with a quorum of 2f+1 votes.
            .find(|(_, v)| *v > 2 * faulty_replicas)
            .map(|(h, _)| h);

        // Get the highest CommitQC and verify it.
        let highest_qc: validator::CommitQC = message
            .justification
            .map
            .keys()
            .max_by_key(|m| m.high_qc.message.view)
            .unwrap()
            .high_qc
            .clone();
        // Check that the proposal is valid.
        match &self.proposal_payload {
            // The leader proposed a new block.
            Some(payload) => {
                // Check that the payload doesn't exceed the maximum size.
                if payload.0.len() > self.config.max_payload_size {
                    return Err(Error::ProposalOversizedPayload {
                        payload_size: payload.0.len(),
                        header: message.proposal,
                    });
                }

                // Check that payload matches the header
                if message.proposal.payload != payload.hash() {
                    return Err(Error::ProposalMismatchedPayload);
                }

                // Check that we finalized the previous block.
                if highest_vote.is_some()
                    && highest_vote.as_ref() != Some(&highest_qc.message.proposal)
                {
                    return Err(Error::ProposalWhenPreviousNotFinalized);
                }

                // Parent hash should match.
                if highest_qc.message.proposal.hash() != message.proposal.parent {
                    return Err(Error::ProposalInvalidParentHash {
                        correct_parent_hash: highest_qc.header().hash(),
                        received_parent_hash: message.proposal.parent,
                        header: message.proposal.clone(),
                    });
                }

                // Block number should match.
                if highest_qc.header().number.next() != message.proposal.number {
                    return Err(Error::ProposalNonSequentialNumber {
                        correct_number: highest_qc.header().number.next(),
                        received_number: message.proposal.number,
                        header: message.proposal,
                    });
                }

        // Block number should match.
        if highest_qc.header().number.next() != message.proposal.number {
            return Err(Error::ProposalNonSequentialNumber {
                correct_number: highest_qc.header().number.next(),
                received_number: message.proposal.number,
                header: message.proposal,
            });
        }
        if let Some(payload) = &self.proposal_payload {
            anyhow::ensure!(self.proposal.payload == payload.hash()); 
        }
        Ok(())
    }
}

/// A Commit message from a leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderCommit {
    /// The CommitQC that justifies the message from the leader.
    pub justification: CommitQC,
}

impl LeaderCommit {
    /// Verifies LeaderCommit.
    pub fn verify(&self, genesis: &Genesis) -> anyhow::Result<()> {
        self.justification.verify(genesis).context("justification")
    }

    /// View of this message.
    pub fn view(&self) -> &View {
        self.justification.view()
    }
}

/// A quorum certificate of replica Prepare messages. Since not all Prepare messages are
/// identical (they have different high blocks and high QCs), we need to keep the high blocks
/// and high QCs in a map. We can still aggregate the signatures though.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrepareQC {
    /// View of this QC.
    pub view: View,
    /// Map from replica Prepare messages to the validators that signed them.
    pub map: BTreeMap<ReplicaPrepare, Signers>,
    /// Aggregate signature of the replica Prepare messages.
    pub signature: validator::AggregateSignature,
}

impl PrepareQC {
    /// Create a new empty instance for a given `ReplicaCommit` message and a validator set size.
    pub fn new(view: View) -> Self {
        Self {
            view,
            map: BTreeMap::new(),
            signature: validator::AggregateSignature::default(),
        }
    }

    /// Add a validator's signed message.
    /// * `signed_message` - A valid signed `ReplicaPrepare` message.
    /// * `validator_index` - The signer index in the validator set.
    pub fn add(&mut self, msg: &Signed<ReplicaPrepare>, genesis: &Genesis) {
        let Some(i) = genesis.validators.index(&msg.key) else { return };
        let e = self.map.entry(msg.msg.clone()).or_insert_with(|| Signers::new(genesis.validators.len()));
        if e.0[i] { return };
        e.0.set(i,true);
        self.signature.add(&msg.sig);
    }

    /// Verifies the integrity of the PrepareQC.
    pub fn verify(&self, genesis: &Genesis) -> anyhow::Result<()> {
        let mut sum = Signers::new(genesis.validators.len());
        // Check the ReplicaPrepare messages.
        for (i,(msg,signers)) in self.map.iter().enumerate() {
            anyhow::ensure!(msg.view == self.view, "msg[{i}].view = {:?}, want {:?}", msg.view, self.view);
            anyhow::ensure!(signers.len() == sum.len(), "Bit vector in PrepareQC has wrong length!");
            anyhow::ensure!(!signers.is_empty(), "msg[{i}] has no signers assigned");
            anyhow::ensure!((&sum & signers).is_empty(),"Bit vectors in PrepareQC are not disjoint. We require that every validator signs at most one message!");
            // TODO(gprusak): this is overly cautious for the replicas to check.
            msg.verify(genesis).with_context(||format!("msg[{i}]"))?;
            sum |= signers;
        }

        // Verify that we have enough signers.
        let threshold = genesis.validators.threshold();
        anyhow::ensure!(sum.count() >= threshold, "Insufficient signers in PrepareQC: got {}, want >= {threshold}",sum.len());

        // Now we can verify the signature.
        let messages_and_keys = self.map.clone().into_iter().flat_map(|(msg, signers)| {
            genesis
                .validators
                .iter()
                .enumerate()
                .filter(|(i, _)| signers.0[*i])
                .map(|(_, pk)| (msg.clone(), pk))
                .collect::<Vec<_>>()
        });
        // TODO(gprusak): This reaggregating is suboptimal.
        Ok(self.signature.verify_messages(messages_and_keys)?)
    }
}

/// A Commit Quorum Certificate. It is an aggregate of signed replica Commit messages.
/// The Quorum Certificate is supposed to be over identical messages, so we only need one message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CommitQC {
    /// The replica Commit message that the QC is for.
    pub message: ReplicaCommit,
    /// The validators that signed this message.
    pub signers: Signers,
    /// The aggregate signature of the signed replica messages.
    pub signature: validator::AggregateSignature,
}

impl CommitQC {
    /// Header of the certified block.
    pub fn header(&self) -> &BlockHeader {
        &self.message.proposal
    }

    /// View of this QC.
    pub fn view(&self) -> &View {
        &self.message.view
    }

    /// Create a new empty instance for a given `ReplicaCommit` message and a validator set size.
    pub fn new(message: ReplicaCommit, genesis: &Genesis) -> Self {
        Self {
            message,
            signers: Signers::new(genesis.validators.len()),
            signature: validator::AggregateSignature::default(),
        }
    }

    /// Add a validator's signature.
    pub fn add(&mut self, msg: Signed<ReplicaCommit>, genesis: &Genesis) {
        if self.message != msg.msg { return };
        let Some(i) = genesis.validators.index(&msg.key) else { return };
        if self.signers.0[i] { return };
        self.signers.0.set(i, true);
        self.signature.add(&msg.sig);
    }

    /// Verifies the signature of the CommitQC.
    pub fn verify(&self, genesis: &Genesis) -> anyhow::Result<()> {
        anyhow::ensure!(self.signers.len() == genesis.validators.len(), "Bit vector in CommitQC has wrong length!");

        // Verify that we have enough signers.
        let num_signers = self.signers.count();
        let threshold = genesis.validators.threshold();
        anyhow::ensure!(num_signers >= threshold, "got {num_signers} signers, want >= {threshold}");

        // Now we can verify the signature.
        let messages_and_keys = genesis.validators
            .iter()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        Ok(self.signature.verify_messages(messages_and_keys)?)
    }
}

/// Struct that represents a bit map of validators. We use it to compactly store
/// which validators signed a given message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Signers(pub BitVec);

impl Signers {
    /// Constructs an empty signers set.
    pub fn new(n: usize) -> Self {
        Self(BitVec::from_elem(n, false))
    }

    /// Returns the number of signers, i.e. the number of validators that signed
    /// the particular message that this signer bitmap refers to.
    pub fn count(&self) -> usize {
        self.0.iter().filter(|b| *b).count()
    }


    /// Size of the corresponding ValidatorSet.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if there are no signers.
    pub fn is_empty(&self) -> bool {
        self.0.none()
    }
}

impl std::ops::BitOrAssign<&Self> for Signers {
    fn bitor_assign(&mut self, other: &Self) {
        self.0.or(&other.0);
    }
}

impl std::ops::BitAndAssign<&Self> for Signers {
    fn bitand_assign(&mut self, other: &Self) {
        self.0.and(&other.0);
    }
}

impl std::ops::BitAnd for &Signers {
    type Output = Signers;
    fn bitand(self, other: Self) -> Signers {
        let mut this = self.clone();
        this &= other;
        this
    }
}

/// A struct that represents a set of validators. It is used to store the current validator set.
/// We represent each validator by its validator public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatorSet {
    vec: Vec<validator::PublicKey>,
    map: BTreeMap<validator::PublicKey, usize>,
}

impl ValidatorSet {
    /// Creates a new ValidatorSet from a list of validator public keys.
    pub fn new(validators: impl IntoIterator<Item = validator::PublicKey>) -> anyhow::Result<Self> {
        let mut set = BTreeSet::new();
        for validator in validators {
            anyhow::ensure!(set.insert(validator),"Duplicate validator in ValidatorSet");
        }
        anyhow::ensure!(!set.is_empty(), "ValidatorSet must contain at least one validator");
        Ok(Self {
            vec: set.iter().cloned().collect(),
            map: set.into_iter().enumerate().map(|(i, pk)| (pk, i)).collect(),
        })
    }

    /// Iterates over validators.
    pub fn iter(&self) -> impl Iterator<Item = &validator::PublicKey> {
        self.vec.iter()
    }

    /// Returns the number of validators.
    #[allow(clippy::len_without_is_empty)] // a valid `ValidatorSet` is always non-empty by construction
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    /// Returns true if the given validator is in the validator set.
    pub fn contains(&self, validator: &validator::PublicKey) -> bool {
        self.map.contains_key(validator)
    }

    /// Get validator by its index in the set.
    pub fn get(&self, index: usize) -> Option<&validator::PublicKey> {
        self.vec.get(index)
    }

    /// Get the index of a validator in the set.
    pub fn index(&self, validator: &validator::PublicKey) -> Option<usize> {
        self.map.get(validator).copied()
    }

    /// Computes the validator for the given view.
    pub fn view_leader(&self, view_number: ViewNumber) -> validator::PublicKey {
        let index = view_number.0 as usize % self.len();
        self.get(index).unwrap().clone()
    }
   
    /// Signature threshold for this validator set.
    pub fn threshold(&self) -> usize {
        threshold(self.len())
    }
   
    /// Maximal number of faulty replicas allowed in this validator set.
    pub fn faulty_replicas(&self) -> usize {
        faulty_replicas(self.len())
    }
}

/// Calculate the consensus threshold, the minimum number of votes for any consensus action to be valid,
/// for a given number of replicas.
pub fn threshold(n: usize) -> usize {
    n-faulty_replicas(n)
}

/// Calculate the maximum number of faulty replicas, for a given number of replicas.
pub fn faulty_replicas(n: usize) -> usize {
    // Calculate the allowed maximum number of faulty replicas. We want the following relationship to hold:
    //      n = 5*f + 1
    // for n total replicas and f faulty replicas. This results in the following formula for the maximum
    // number of faulty replicas:
    //      f = floor((n - 1) / 5)
    // Because of this, it doesn't make sense to have 5*f + 2 or 5*f + 3 replicas. It won't increase the number
    // of allowed faulty replicas.
    (n - 1) / 5
}

/// A struct that represents a view number.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewNumber(pub u64);

impl ViewNumber {
    /// Get the next view number.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Get the previous view number.
    pub fn prev(self) -> Self {
        Self(self.0 - 1)
    }
}

/// An enum that represents the current phase of the consensus.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Prepare,
    Commit,
}
