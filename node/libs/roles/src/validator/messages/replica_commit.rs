use super::{BlockHeader, Genesis, View};

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
        use ReplicaCommitVerifyError as Error;
        self.view.verify(genesis).map_err(Error::View)?;
        if self.proposal.number < genesis.first_block {
            return Err(Error::BadBlockNumber);
        }
        Ok(())
    }
}
