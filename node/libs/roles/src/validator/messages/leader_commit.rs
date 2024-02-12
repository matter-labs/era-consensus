use super::*;
use crate::validator;

/// A Commit message from a leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderCommit {
    /// The CommitQC that justifies the message from the leader.
    pub justification: CommitQC,
}

impl LeaderCommit {
    /// Verifies LeaderCommit.
    pub fn verify(&self, genesis: &Genesis) -> Result<(),CommitQCVerifyError> {
        self.justification.verify(genesis,/*allow_past_forks=*/false)
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
#[derive(thiserror::Error,Debug)]
pub enum CommitQCVerifyError {
    /// Invalid message.
    #[error("invalid message: {0:#}")]
    InvalidMessage(#[source] anyhow::Error),
    /// Bad signer set.
    #[error("signers set doesn't match genesis")]
    BadSignersSet,
    /// Not enough signers.
    #[error("not enough signers: got {got}, want {want}")]
    NotEnoughSigners { 
        /// Got signers.
        got: usize,
        /// Want signers.
        want: usize,
    }, 
    /// Bad signature.
    #[error("bad signature: {0:#}")]
    BadSignature(#[source] validator::Error),
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
    pub fn add(&mut self, msg: &Signed<ReplicaCommit>, genesis: &Genesis) {
        if self.message != msg.msg { return };
        let Some(i) = genesis.validators.index(&msg.key) else { return };
        if self.signers.0[i] { return };
        self.signers.0.set(i, true);
        self.signature.add(&msg.sig);
    }

    /// Verifies the signature of the CommitQC.
    pub fn verify(&self, genesis: &Genesis, allow_past_forks: bool) -> Result<(),CommitQCVerifyError> {
        use CommitQCVerifyError as Error;
        self.message.verify(genesis, allow_past_forks).map_err(Error::InvalidMessage)?;
        if self.signers.len() != genesis.validators.len() {
            return Err(Error::BadSignersSet);
        }

        // Verify that we have enough signers.
        let num_signers = self.signers.count();
        let threshold = genesis.validators.threshold();
        if num_signers < threshold {
            return Err(Error::NotEnoughSigners { got: num_signers, want: threshold });
        }

        // Now we can verify the signature.
        let messages_and_keys = genesis.validators
            .iter()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature.verify_messages(messages_and_keys).map_err(Error::BadSignature)
    }
}


