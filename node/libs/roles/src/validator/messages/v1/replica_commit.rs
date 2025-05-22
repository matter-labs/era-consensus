use anyhow::Context as _;
use zksync_protobuf::{read_required, ProtoFmt};

use super::{get_committee_from_schedule, BlockHeader, Signers, View};
use crate::{
    proto::validator as proto,
    validator::{self, GenesisHash, Signed},
};

/// A commit message from a replica.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicaCommit {
    /// View of this message.
    pub view: View,
    /// The header of the block that the replica is committing to.
    pub proposal: BlockHeader,
}

impl ReplicaCommit {
    /// Verifies the message.
    pub fn verify(&self, genesis_hash: GenesisHash) -> Result<(), ReplicaCommitVerifyError> {
        self.view
            .verify(genesis_hash)
            .map_err(ReplicaCommitVerifyError::BadView)?;

        Ok(())
    }
}

impl ProtoFmt for ReplicaCommit {
    type Proto = proto::ReplicaCommit;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view: read_required(&r.view).context("view")?,
            proposal: read_required(&r.proposal).context("proposal")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            view: Some(self.view.build()),
            proposal: Some(self.proposal.build()),
        }
    }
}

/// Error returned by `ReplicaCommit::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum ReplicaCommitVerifyError {
    /// Invalid view.
    #[error("view: {0:#}")]
    BadView(anyhow::Error),
}

/// A Commit Quorum Certificate. It is an aggregate of signed ReplicaCommit messages.
/// The Commit Quorum Certificate is over identical messages, so we only need one message.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CommitQC {
    /// The ReplicaCommit message that the QC is for.
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
    pub fn new(message: ReplicaCommit, validators: &validator::Schedule) -> Self {
        Self {
            message,
            signers: Signers::new(validators.len()),
            signature: validator::AggregateSignature::default(),
        }
    }

    /// Add a validator's signature. This also verifies the message and the signature before adding.
    pub fn add(
        &mut self,
        msg: &Signed<ReplicaCommit>,
        genesis_hash: GenesisHash,
        validators: &validator::Schedule,
    ) -> Result<(), CommitQCAddError> {
        // Check if the signer is in the committee.
        let Some(i) = validators.index(&msg.key) else {
            return Err(CommitQCAddError::SignerNotInCommittee {
                signer: Box::new(msg.key.clone()),
            });
        };

        // Check if already have a message from the same signer.
        if self.signers.0[i] {
            return Err(CommitQCAddError::DuplicateSigner {
                signer: Box::new(msg.key.clone()),
            });
        };

        // Verify the signature.
        msg.verify().map_err(CommitQCAddError::BadSignature)?;

        // Check that the message is consistent with the CommitQC.
        if self.message != msg.msg {
            return Err(CommitQCAddError::InconsistentMessages);
        };

        // Check that the message itself is valid.
        msg.msg
            .verify(genesis_hash)
            .map_err(CommitQCAddError::InvalidMessage)?;

        // Add the signer to the signers map, and the signature to the aggregate signature.
        self.signers.0.set(i, true);
        self.signature.add(&msg.sig);

        Ok(())
    }

    /// Verifies the integrity of the CommitQC.
    pub fn verify(
        &self,
        genesis_hash: GenesisHash,
        validators: &validator::Schedule,
    ) -> Result<(), CommitQCVerifyError> {
        // Check that the message is valid.
        self.message
            .verify(genesis_hash)
            .map_err(CommitQCVerifyError::InvalidMessage)?;

        // Check that the signers set has the same size as the validator set.
        if self.signers.len() != validators.len() {
            return Err(CommitQCVerifyError::BadSignersSet);
        }

        // Verify the signers' weight is enough.
        let weight = self
            .signers
            .weight(&get_committee_from_schedule(validators));
        let threshold = validators.quorum_threshold();
        if weight < threshold {
            return Err(CommitQCVerifyError::NotEnoughWeight {
                got: weight,
                want: threshold,
            });
        }

        // Now we can verify the signature.
        let messages_and_keys = validators
            .keys()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature
            .verify_messages(messages_and_keys)
            .map_err(CommitQCVerifyError::BadSignature)
    }
}

impl ProtoFmt for CommitQC {
    type Proto = proto::CommitQc;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            message: read_required(&r.msg).context("msg")?,
            signers: read_required(&r.signers).context("signers")?,
            signature: read_required(&r.sig).context("sig")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            msg: Some(self.message.build()),
            signers: Some(self.signers.build()),
            sig: Some(self.signature.build()),
        }
    }
}

/// Error returned by `CommitQC::add()`.
#[derive(thiserror::Error, Debug)]
pub enum CommitQCAddError {
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
    /// Inconsistent messages.
    #[error("Trying to add signature for a different message")]
    InconsistentMessages,
    /// Invalid message.
    #[error("Invalid message: {0:#}")]
    InvalidMessage(ReplicaCommitVerifyError),
}

/// Error returned by `CommitQC::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum CommitQCVerifyError {
    /// Invalid message.
    #[error(transparent)]
    InvalidMessage(#[from] ReplicaCommitVerifyError),
    /// Bad signer set.
    #[error("Signers set doesn't match validator set")]
    BadSignersSet,
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
