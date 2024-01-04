//! Messages related to the consensus protocol.

use super::{BlockHeader, Msg, Payload, Signed};
use crate::{validator, validator::Signature};
use anyhow::bail;
use bit_vec::BitVec;
use std::collections::{BTreeMap, BTreeSet};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};

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
