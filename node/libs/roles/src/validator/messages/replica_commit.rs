use super::{BlockHeader, Genesis, Signed, Signers, View};
use crate::validator;

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
    pub fn verify(&self, genesis: &Genesis) -> Result<(), ReplicaCommitVerifyError> {
        self.view
            .verify(genesis)
            .map_err(ReplicaCommitVerifyError::View)?;

        if self.proposal.number < genesis.first_block {
            return Err(ReplicaCommitVerifyError::BadBlockNumber);
        }

        Ok(())
    }
}

/// Error returned by `ReplicaCommit::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum ReplicaCommitVerifyError {
    /// Invalid view.
    #[error("view: {0:#}")]
    View(anyhow::Error),
    /// Bad block number.
    #[error("block number < first block")]
    BadBlockNumber,
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
    /// Signature is assumed to be already verified.
    pub fn add(
        &mut self,
        msg: &Signed<ReplicaCommit>,
        genesis: &Genesis,
    ) -> Result<(), CommitQCAddError> {
        if self.message != msg.msg {
            return Err(CommitQCAddError::InconsistentMessages);
        };

        let Some(i) = genesis.validators.index(&msg.key) else {
            return Err(CommitQCAddError::SignerNotInCommittee {
                signer: Box::new(msg.key.clone()),
            });
        };

        if self.signers.0[i] {
            return Err(CommitQCAddError::Exists);
        };

        self.signers.0.set(i, true);
        self.signature.add(&msg.sig);

        Ok(())
    }

    /// Verifies the signature of the CommitQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), CommitQCVerifyError> {
        self.message
            .verify(genesis)
            .map_err(CommitQCVerifyError::InvalidMessage)?;

        if self.signers.len() != genesis.validators.len() {
            return Err(CommitQCVerifyError::BadSignersSet);
        }

        // Verify the signers' weight is enough.
        let weight = genesis.validators.weight(&self.signers);
        let threshold = genesis.validators.threshold();
        if weight < threshold {
            return Err(CommitQCVerifyError::NotEnoughSigners {
                got: weight,
                want: threshold,
            });
        }

        // Now we can verify the signature.
        let messages_and_keys = genesis
            .validators
            .keys()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature
            .verify_messages(messages_and_keys)
            .map_err(CommitQCVerifyError::BadSignature)
    }
}

/// Error returned by `CommitQc::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum CommitQCVerifyError {
    /// Invalid message.
    #[error(transparent)]
    InvalidMessage(#[from] ReplicaCommitVerifyError),
    /// Bad signer set.
    #[error("signers set doesn't match genesis")]
    BadSignersSet,
    /// Weight not reached.
    #[error("Signers have not reached threshold weight: got {got}, want {want}")]
    NotEnoughSigners {
        /// Got weight.
        got: u64,
        /// Want weight.
        want: u64,
    },
    /// Bad signature.
    #[error("bad signature: {0:#}")]
    BadSignature(#[source] anyhow::Error),
}

/// Error returned by `CommitQC::add()`.
#[derive(thiserror::Error, Debug)]
pub enum CommitQCAddError {
    /// Inconsistent messages.
    #[error("Trying to add signature for a different message")]
    InconsistentMessages,
    /// Signer not present in the committee.
    #[error("Signer not in committee: {signer:?}")]
    SignerNotInCommittee {
        /// Signer of the message.
        signer: Box<validator::PublicKey>,
    },
    /// Message already present in CommitQC.
    #[error("Message already signed for CommitQC")]
    Exists,
}
