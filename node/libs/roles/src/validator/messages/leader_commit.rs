use super::{BlockHeader, Genesis, ReplicaCommit, Signed, Signers, View};
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
    #[error("invalid message: {0:#}")]
    InvalidMessage(#[source] anyhow::Error),
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
    pub fn add(&mut self, msg: &Signed<ReplicaCommit>, genesis: &Genesis) {
        if self.message != msg.msg {
            return;
        };
        let Some(i) = genesis.validators.index(&msg.key) else {
            return;
        };
        if self.signers.0[i] {
            return;
        };
        self.signers.0.set(i, true);
        self.signature.add(&msg.sig);
    }

    /// Verifies the signature of the CommitQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), CommitQCVerifyError> {
        use CommitQCVerifyError as Error;
        self.message
            .verify(genesis)
            .map_err(Error::InvalidMessage)?;
        if self.signers.len() != genesis.validators.len() {
            return Err(Error::BadSignersSet);
        }

        // Verify the signers' weight is enough.
        let weight = genesis.validators.weight(&self.signers);
        let threshold = genesis.validators.threshold();
        if weight < threshold {
            return Err(Error::NotEnoughSigners {
                got: weight,
                want: threshold,
            });
        }

        // Now we can verify the signature.
        let messages_and_keys = genesis
            .validators
            .iter_keys()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature
            .verify_messages(messages_and_keys)
            .map_err(Error::BadSignature)
    }
}
