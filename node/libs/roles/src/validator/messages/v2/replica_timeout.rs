use std::collections::{BTreeMap, HashMap};

use anyhow::Context as _;
use zksync_protobuf::{read_optional, read_required, ProtoFmt};

use super::{
    BlockHeader, CommitQC, CommitQCVerifyError, ReplicaCommit, ReplicaCommitVerifyError, Signers,
    View,
};
use crate::{
    proto::validator as proto,
    validator::{self, EpochNumber, GenesisHash, Signed},
};

/// A timeout message from a replica.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReplicaTimeout {
    /// View of this message.
    pub view: View,
    /// The highest block that the replica has committed to.
    pub high_vote: Option<ReplicaCommit>,
    /// The highest CommitQC that the replica has seen.
    pub high_qc: Option<CommitQC>,
}

impl ReplicaTimeout {
    /// Verifies the message.
    pub fn verify(
        &self,
        genesis_hash: GenesisHash,
        epoch: EpochNumber,
        validators_schedule: &validator::Schedule,
    ) -> Result<(), ReplicaTimeoutVerifyError> {
        self.view
            .verify(genesis_hash, epoch)
            .map_err(ReplicaTimeoutVerifyError::BadView)?;

        if let Some(v) = &self.high_vote {
            v.verify(genesis_hash, epoch)
                .map_err(ReplicaTimeoutVerifyError::InvalidHighVote)?;
        }

        if let Some(qc) = &self.high_qc {
            qc.verify(genesis_hash, epoch, validators_schedule)
                .map_err(ReplicaTimeoutVerifyError::InvalidHighQC)?;
        }

        Ok(())
    }
}

impl ProtoFmt for ReplicaTimeout {
    type Proto = proto::ReplicaTimeoutV2;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view: read_required(&r.view).context("view")?,
            high_vote: read_optional(&r.high_vote).context("high_vote")?,
            high_qc: read_optional(&r.high_qc).context("high_qc")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            view: Some(self.view.build()),
            high_vote: self.high_vote.as_ref().map(ProtoFmt::build),
            high_qc: self.high_qc.as_ref().map(ProtoFmt::build),
        }
    }
}

/// Error returned by `ReplicaTimeout::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum ReplicaTimeoutVerifyError {
    /// View.
    #[error("view: {0:#}")]
    BadView(anyhow::Error),
    /// Invalid High Vote.
    #[error("invalid high_vote: {0:#}")]
    InvalidHighVote(ReplicaCommitVerifyError),
    /// Invalid High QC.
    #[error("invalid high_qc: {0:#}")]
    InvalidHighQC(CommitQCVerifyError),
}

/// A quorum certificate of ReplicaTimeout messages. Since not all ReplicaTimeout messages are
/// identical (they have different high blocks and high QCs), we need to keep the ReplicaTimeout
/// messages in a map. We can still aggregate the signatures though.
///
/// WARNING: `TimeoutQC` message contains a map indexed by `ReplicaTimeout` messages.
/// Therefore, `Ord` implementation of `ReplicaTimeout` affects the unique encoding of the `TimeoutQC` message.
/// This `Ord` implementation is the derived lexicographic ordering of the fields of
/// `ReplicaTimeout` (and transitively on types of the fields AS WELL).
/// As a result ANY change to type of ANY transitive field of ReplicaTimeout struct may invalidate
/// preexisting signatures of `TimeoutQC` messages (or messages containing `TimeoutQC`).
///
/// The proper fix would be to add support for unordered collections on the protobuf level
/// (for example, by adding a custom option "unordered" for repeated fields, which would make the encoder
/// sort the encoded elements before encoding the whole message). However the current protobuf
/// encoding of TimeoutQC keeps the keys and values in separate repeated fields. Until this is
/// fixed, ANY change to the above mentioned types may be backward incompatible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeoutQC {
    /// View of this QC.
    pub view: View,
    /// Map from replica Timeout messages to the validators that signed them.
    pub map: BTreeMap<ReplicaTimeout, Signers>,
    /// Aggregate signature of the ReplicaTimeout messages.
    pub signature: validator::AggregateSignature,
}

impl TimeoutQC {
    /// Create a new empty TimeoutQC for a given view.
    pub fn new(view: View) -> Self {
        Self {
            view,
            map: BTreeMap::new(),
            signature: validator::AggregateSignature::default(),
        }
    }

    /// Get the highest block voted and check if there's a subquorum of votes for it. To have a subquorum
    /// in this situation, we require n-3*f votes, where f is the maximum number of faulty replicas.
    /// Note that it is possible to have 2 subquorums: vote A and vote B, each with >n-3*f weight, in a single
    /// TimeoutQC. In such a situation we say that there is no high vote.
    pub fn high_vote(&self, validators_schedule: &validator::Schedule) -> Option<BlockHeader> {
        let mut count: HashMap<_, u64> = HashMap::new();
        for (msg, signers) in &self.map {
            if let Some(v) = &msg.high_vote {
                *count.entry(v.proposal).or_default() += signers.weight(validators_schedule);
            }
        }

        let min = validators_schedule.subquorum_threshold();
        let mut high_votes: Vec<_> = count.into_iter().filter(|x| x.1 >= min).collect();

        if high_votes.len() == 1 {
            high_votes.pop().map(|x| x.0)
        } else {
            None
        }
    }

    /// Get the highest CommitQC.
    pub fn high_qc(&self) -> Option<&CommitQC> {
        self.map
            .keys()
            .filter_map(|m| m.high_qc.as_ref())
            .max_by_key(|qc| qc.view().number)
    }

    /// Add a validator's signed message. This also verifies the message and the signature before adding.
    pub fn add(
        &mut self,
        msg: &Signed<ReplicaTimeout>,
        genesis_hash: GenesisHash,
        epoch: EpochNumber,
        validators_schedule: &validator::Schedule,
    ) -> Result<(), TimeoutQCAddError> {
        // Check if the signer is in the committee.
        let Some(i) = validators_schedule.index(&msg.key) else {
            return Err(TimeoutQCAddError::SignerNotInCommittee {
                signer: Box::new(msg.key.clone()),
            });
        };

        // Check if we already have a message from the same signer.
        if self.map.values().any(|s| s.0[i]) {
            return Err(TimeoutQCAddError::DuplicateSigner {
                signer: Box::new(msg.key.clone()),
            });
        };

        // Verify the signature.
        msg.verify().map_err(TimeoutQCAddError::BadSignature)?;

        // Check that the view is consistent with the TimeoutQC.
        if msg.msg.view != self.view {
            return Err(TimeoutQCAddError::InconsistentViews);
        };

        // Check that the message itself is valid.
        msg.msg
            .verify(genesis_hash, epoch, validators_schedule)
            .map_err(TimeoutQCAddError::InvalidMessage)?;

        // Add the message plus signer to the map, and the signature to the aggregate signature.
        let e = self
            .map
            .entry(msg.msg.clone())
            .or_insert_with(|| Signers::new(validators_schedule.len()));
        e.0.set(i, true);
        self.signature.add(&msg.sig);

        Ok(())
    }

    /// Verifies the integrity of the TimeoutQC.
    pub fn verify(
        &self,
        genesis_hash: GenesisHash,
        epoch: EpochNumber,
        validators_schedule: &validator::Schedule,
    ) -> Result<(), TimeoutQCVerifyError> {
        self.view
            .verify(genesis_hash, epoch)
            .map_err(TimeoutQCVerifyError::BadView)?;

        let mut sum = Signers::new(validators_schedule.len());

        // Check the ReplicaTimeout messages.
        for (i, (msg, signers)) in self.map.iter().enumerate() {
            if msg.view != self.view {
                return Err(TimeoutQCVerifyError::InconsistentView(i));
            }
            if signers.len() != sum.len() {
                return Err(TimeoutQCVerifyError::WrongSignersLength(i));
            }
            if signers.is_empty() {
                return Err(TimeoutQCVerifyError::NoSignersAssigned(i));
            }
            if !(&sum & signers).is_empty() {
                return Err(TimeoutQCVerifyError::OverlappingSignatureSet(i));
            }
            msg.verify(genesis_hash, epoch, validators_schedule)
                .map_err(|err| TimeoutQCVerifyError::InvalidMessage(i, err))?;

            sum |= signers;
        }

        // Check if the signers' weight is enough.
        let weight = sum.weight(validators_schedule);
        let threshold = validators_schedule.quorum_threshold();
        if weight < threshold {
            return Err(TimeoutQCVerifyError::NotEnoughWeight {
                got: weight,
                want: threshold,
            });
        }

        // Now we can verify the signature.
        let messages_and_keys = self.map.clone().into_iter().flat_map(|(msg, signers)| {
            validators_schedule
                .keys()
                .enumerate()
                .filter(|(i, _)| signers.0[*i])
                .map(|(_, pk)| (msg.clone(), pk))
                .collect::<Vec<_>>()
        });

        // TODO: This reaggregating is suboptimal.
        self.signature
            .verify_messages(messages_and_keys)
            .map_err(TimeoutQCVerifyError::BadSignature)
    }

    /// Calculates the weight of current TimeoutQC signing validators
    pub fn weight(&self, validators_schedule: &validator::Schedule) -> u64 {
        self.map
            .values()
            .map(|signers| signers.weight(validators_schedule))
            .sum()
    }
}

impl Ord for TimeoutQC {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.view.number.cmp(&other.view.number)
    }
}

impl PartialOrd for TimeoutQC {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl ProtoFmt for TimeoutQC {
    type Proto = proto::TimeoutQcv2;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut map = BTreeMap::new();

        for (msg, signers) in r.msgs.iter().zip(r.signers.iter()) {
            map.insert(
                ReplicaTimeout::read(msg).context("msg")?,
                Signers::read(signers).context("signers")?,
            );
        }

        Ok(Self {
            view: read_required(&r.view).context("view")?,
            map,
            signature: read_required(&r.sig).context("sig")?,
        })
    }

    fn build(&self) -> Self::Proto {
        let (msgs, signers) = self
            .map
            .iter()
            .map(|(msg, signers)| (msg.build(), signers.build()))
            .unzip();

        Self::Proto {
            view: Some(self.view.build()),
            msgs,
            signers,
            sig: Some(self.signature.build()),
        }
    }
}

/// Error returned by `TimeoutQC::add()`.
#[derive(thiserror::Error, Debug)]
pub enum TimeoutQCAddError {
    /// Signer not present in the committee.
    #[error("Signer not in committee: {signer:?}")]
    SignerNotInCommittee {
        /// Signer of the message.
        signer: Box<validator::PublicKey>,
    },
    /// Message from the same signer already present in QC.
    #[error("Message from the same signer already in QC: {signer:?}")]
    DuplicateSigner {
        /// Signer of the message.
        signer: Box<validator::PublicKey>,
    },
    /// Bad signature.
    #[error("Bad signature: {0:#}")]
    BadSignature(#[source] anyhow::Error),
    /// Inconsistent views.
    #[error("Trying to add a message from a different view")]
    InconsistentViews,
    /// Invalid message.
    #[error("Invalid message: {0:#}")]
    InvalidMessage(ReplicaTimeoutVerifyError),
}

/// Error returned by `TimeoutQC::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum TimeoutQCVerifyError {
    /// Bad view.
    #[error("Bad view: {0:#}")]
    BadView(anyhow::Error),
    /// Inconsistent views.
    #[error("Message with inconsistent view: number [{0}]")]
    InconsistentView(usize),
    /// Invalid message.
    #[error("Invalid message: number [{0}], {1:#}")]
    InvalidMessage(usize, ReplicaTimeoutVerifyError),
    /// Wrong signers length.
    #[error("Message with wrong signers length: number [{0}]")]
    WrongSignersLength(usize),
    /// No signers assigned.
    #[error("Message with no signers assigned: number [{0}]")]
    NoSignersAssigned(usize),
    /// Overlapping signature sets.
    #[error("Message with overlapping signature set: number [{0}]")]
    OverlappingSignatureSet(usize),
    /// Weight not reached.
    #[error("Signers have not reached threshold weight: got {got}, want {want}")]
    NotEnoughWeight {
        /// Got weight.
        got: u64,
        /// Want weight.
        want: u64,
    },
    /// Bad signature.
    #[error("Bad signature: {0:#}")]
    BadSignature(#[source] anyhow::Error),
}
