use super::{CommitQC, CommitQCVerifyError, ForkNumber, Genesis, ReplicaCommit, View};

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

/// Error returned by `ReplicaPrepare::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum ReplicaPrepareVerifyError {
    /// BadFork.
    #[error("bad fork: got {got:?}, want {want:?}")]
    BadFork {
        /// got
        got: ForkNumber,
        /// want
        want: ForkNumber,
    },
    /// FutureHighVoteView.
    #[error("high vote from the future")]
    HighVoteFutureView,
    /// FutureHighQCView.
    #[error("high qc from the future")]
    HighQCFutureView,
    /// HighVote.
    #[error("high_vote: {0:#}")]
    HighVote(anyhow::Error),
    /// HighQC.
    #[error("high_qc: {0:#}")]
    HighQC(CommitQCVerifyError),
}

impl ReplicaPrepare {
    /// Verifies the message.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), ReplicaPrepareVerifyError> {
        use ReplicaPrepareVerifyError as Error;
        if self.view.fork != genesis.fork.number {
            return Err(Error::BadFork {
                got: self.view.fork,
                want: genesis.fork.number,
            });
        }
        if let Some(v) = &self.high_vote {
            if self.view.number <= v.view.number {
                return Err(Error::HighVoteFutureView);
            }
            v.verify(genesis)
                .map_err(Error::HighVote)?;
        }
        if let Some(qc) = &self.high_qc {
            if self.view.number <= qc.view().number {
                return Err(Error::HighQCFutureView);
            }
            qc.verify(genesis)
                .map_err(Error::HighQC)?;
        }
        Ok(())
    }
}
