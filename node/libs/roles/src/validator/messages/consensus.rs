//! Messages related to the consensus protocol.
use super::{BlockNumber, BlockHeader, Msg, Payload, Signed};
use crate::{validator, validator::Signature};
use anyhow::bail;
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

    /// Protocol version of this message.
    pub fn protocol_version(&self) -> ProtocolVersion {
        match self {
            Self::ReplicaPrepare(m) => m.protocol_version,
            Self::ReplicaCommit(m) => m.protocol_version,
            Self::LeaderPrepare(m) => m.protocol_version,
            Self::LeaderCommit(m) => m.protocol_version,
        }
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

/// A Prepare message from a replica.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReplicaPrepare {
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// The number of the current view.
    pub view: ViewNumber,
    /// The highest block that the replica has committed to.
    pub high_vote: ReplicaCommit,
    /// The highest CommitQC that the replica has seen.
    pub high_qc: CommitQC,
}

/// A Commit message from a replica.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicaCommit {
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// The number of the current view.
    pub view: ViewNumber,
    /// The header of the block that the replica is committing to.
    pub proposal: BlockHeader,
}

/// A Prepare message from a leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderPrepare {
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// The number of the current view.
    pub view: ViewNumber,
    /// The header of the block that the leader is proposing.
    pub proposal: BlockHeader,
    /// Payload of the block that the leader is proposing.
    /// `None` iff this is a reproposal.
    pub proposal_payload: Option<Payload>,
    /// The PrepareQC that justifies this proposal from the leader.
    pub justification: PrepareQC,
}

/// A Commit message from a leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderCommit {
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// The CommitQC that justifies the message from the leader.
    pub justification: CommitQC,
}

/// A quorum certificate of replica Prepare messages. Since not all Prepare messages are
/// identical (they have different high blocks and high QCs), we need to keep the high blocks
/// and high QCs in a map. We can still aggregate the signatures though.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PrepareQC {
    /// Map from replica Prepare messages to the validators that signed them.
    pub map: BTreeMap<ReplicaPrepare, Signers>,
    /// Aggregate signature of the replica Prepare messages.
    pub signature: validator::AggregateSignature,
}

impl PrepareQC {
    /// View of the QC.
    pub fn view(&self) -> ViewNumber {
        self.map
            .keys()
            .map(|k| k.view)
            .next()
            .unwrap_or(ViewNumber(0))
    }

    /// Add a validator's signed message.
    /// * `signed_message` - A valid signed `ReplicaPrepare` message.
    /// * `validator_index` - The signer index in the validator set.
    /// * `validator_set` - The validator set.
    pub fn add(
        &mut self,
        signed_message: &Signed<ReplicaPrepare>,
        validator_index: usize,
        validator_set: &ValidatorSet,
    ) {
        self.map
            .entry(signed_message.msg.clone())
            .or_insert_with(|| Signers(BitVec::from_elem(validator_set.len(), false)))
            .0
            .set(validator_index, true);

        self.signature.add(&signed_message.sig);
    }

    /// Verifies the integrity of the PrepareQC.
    pub fn verify(
        &self,
        view: ViewNumber,
        validators: &ValidatorSet,
        threshold: usize,
    ) -> anyhow::Result<()> {
        // First we check that all messages are for the same view number.
        for msg in self.map.keys() {
            if msg.view != view {
                bail!("PrepareQC contains messages for different views!");
            }
        }

        // Then we need to do some checks on the signers bit maps.
        let mut bit_map = BitVec::from_elem(validators.len(), false);
        let mut num_signers = 0;

        for signer_bitmap in self.map.values() {
            let signers = signer_bitmap.0.clone();

            if signers.len() != validators.len() {
                bail!("Bit vector in PrepareQC has wrong length!");
            }

            if !signers.any() {
                bail!("Empty bit vector in PrepareQC. We require at least one signer for every message!");
            }

            let mut intersection = bit_map.clone();
            intersection.and(&signers);
            if intersection.any() {
                bail!("Bit vectors in PrepareQC are not disjoint. We require that every validator signs at most one message!");
            }
            bit_map.or(&signers);

            num_signers += signers.iter().filter(|b| *b).count();
        }

        // Verify that we have enough signers.
        // TODO(gprusak): how about num_signers == threshold to make the certificates more uniform?
        if num_signers < threshold {
            bail!(
                "Insufficient signers in PrepareQC.\nNumber of signers: {}\nThreshold: {}",
                num_signers,
                threshold
            );
        }

        // Now we can verify the signature.
        let messages_and_keys = self.map.clone().into_iter().flat_map(|(msg, signers)| {
            validators
                .iter()
                .enumerate()
                .filter(|(i, _)| signers.0[*i])
                .map(|(_, pk)| (msg.clone(), pk))
                .collect::<Vec<_>>()
        });

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

    /// Create a new empty instance for a given `ReplicaCommit` message and a validator set size.
    pub fn new(message: ReplicaCommit, validator_set: &ValidatorSet) -> Self {
        Self {
            message,
            signers: Signers(BitVec::from_elem(validator_set.len(), false)),
            signature: validator::AggregateSignature::default(),
        }
    }

    /// Add a validator's signature.
    /// * `sig` - A valid signature.
    /// * `validator_index` - The signer index in the validator set.
    pub fn add(&mut self, sig: &Signature, validator_index: usize) {
        self.signers.0.set(validator_index, true);
        self.signature.add(sig);
    }

    /// Verifies the signature of the CommitQC.
    pub fn verify(&self, validators: &ValidatorSet, threshold: usize) -> anyhow::Result<()> {
        let signers = self.signers.0.clone();

        // First we to do some checks on the signers bit map.
        if signers.len() != validators.len() {
            bail!("Bit vector in CommitQC has wrong length!");
        }

        if !signers.any() {
            bail!("Empty bit vector in CommitQC. We require at least one signer!");
        }

        // Verify that we have enough signers.
        let num_signers = signers.iter().filter(|b| *b).count();

        if num_signers < threshold {
            bail!(
                "Insufficient signers in CommitQC.\nNumber of signers: {}\nThreshold: {}",
                num_signers,
                threshold
            );
        }

        // Now we can verify the signature.
        let messages_and_keys = validators
            .iter()
            .enumerate()
            .filter(|(i, _)| signers[*i])
            .map(|(_, pk)| (self.message, pk));

        Ok(self.signature.verify_messages(messages_and_keys)?)
    }
}

/// Struct that represents a bit map of validators. We use it to compactly store
/// which validators signed a given message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Signers(pub BitVec);

impl Signers {
    /// Returns  the number of signers, i.e. the number of validators that signed
    /// the particular message that this signer bitmap refers to.
    pub fn len(&self) -> usize {
        self.0.iter().filter(|b| *b).count()
    }

    /// Returns true if there are no signers.
    pub fn is_empty(&self) -> bool {
        self.0.none()
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
            if !set.insert(validator) {
                bail!("Duplicate validator in ValidatorSet");
            }
        }

        if set.is_empty() {
            bail!("ValidatorSet must contain at least one validator");
        }

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
