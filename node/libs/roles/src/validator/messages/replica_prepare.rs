use super::*;

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
#[derive(thiserror::Error,Debug)]
pub enum ReplicaPrepareVerifyError {
    /// BadFork.
    #[error("bad fork: got {got:?}, want {want:?}")]
    BadFork{
        /// got
        got: ForkId,
        /// want
        want: ForkId,
    },
    /// FutureHighVoteView.
    #[error("high vote from the future")]
    FutureHighVoteView,
    /// FutureHighQCView.
    #[error("high qc from the future")]
    FutureHighQCView,
    /// HighVote.
    #[error("high_vote: {0:#}")]
    HighVote(anyhow::Error),
    /// HighQC.
    #[error("high_qc: {0:#}")]
    HighQC(CommitQCVerifyError),
}

impl ReplicaPrepare {
    /// Verifies the message.
    pub fn verify(&self, genesis: &Genesis) -> Result<(),ReplicaPrepareVerifyError> {
        use ReplicaPrepareVerifyError as Error;
        if self.view.fork != genesis.forks.current() {
            return Err(Error::BadFork { got: self.view.fork, want: genesis.forks.current() });
        }
        if let Some(v) = &self.high_vote {
            if self.view.number <= v.view.number {
                return Err(Error::FutureHighVoteView);
            }
            v.verify(genesis,/*allow_past_forks=*/false).map_err(Error::HighVote)?;
        }
        if let Some(qc) = &self.high_qc {
            if self.view.number <= qc.view().number {
                return Err(Error::FutureHighQCView);
            }
            qc.verify(genesis,/*allow_past_forks=*/false).map_err(Error::HighQC)?;
        }
        Ok(())
    }
}


