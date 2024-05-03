use super::{BlockHeader, Genesis, ReplicaCommit, ReplicaCommitVerifyError, Signed, Signers, View};
use crate::validator;

/// A Commit message from a leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderCommit {
    /// The CommitQC that justifies the message from the leader.
    pub justification: CommitQC,
}

impl LeaderCommit {
    /// Verifies LeaderCommit.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), CommitQCVerifyError> {
        self.justification.verify(genesis)
    }

    /// View of this message.
    pub fn view(&self) -> &View {
        self.justification.view()
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
            signers: Signers::new(genesis.validators_committee.len()),
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
        use CommitQCAddError as Error;
        if self.message != msg.msg {
            return Err(Error::InconsistentMessages);
        };
        let Some(i) = genesis.validators_committee.index(&msg.key) else {
            return Err(Error::SignerNotInCommittee {
                signer: Box::new(msg.key.clone()),
            });
        };
        if self.signers.0[i] {
            return Err(Error::Exists);
        };
        self.signers.0.set(i, true);
        self.signature.add(&msg.sig);
        Ok(())
    }

    /// Verifies the signature of the CommitQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), CommitQCVerifyError> {
        use CommitQCVerifyError as Error;
        self.message
            .verify(genesis)
            .map_err(Error::InvalidMessage)?;
        if self.signers.len() != genesis.validators_committee.len() {
            return Err(Error::BadSignersSet);
        }

        // Verify the signers' weight is enough.
        let weight = genesis.validators_committee.weight(&self.signers);
        let threshold = genesis.validators_committee.threshold();
        if weight < threshold {
            return Err(Error::NotEnoughSigners {
                got: weight,
                want: threshold,
            });
        }

        // Now we can verify the signature.
        let messages_and_keys = genesis
            .validators_committee
            .keys()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature
            .verify_messages(messages_and_keys)
            .map_err(Error::BadSignature)
    }
}
