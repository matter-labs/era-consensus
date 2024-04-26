use super::{BlockHeader, Genesis, View};
use anyhow::Context as _;

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
    pub fn verify(&self, genesis: &Genesis) -> anyhow::Result<()> {
        self.view.verify(genesis).context("view")?;
        anyhow::ensure!(self.proposal.number >= genesis.fork.first_block);
        Ok(())
    }
}
