use super::{BlockHeader, Genesis, View};

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
        anyhow::ensure!(self.view.fork == genesis.fork.number);
        anyhow::ensure!(self.proposal.number >= genesis.fork.first_block);
        if self.proposal.number == genesis.fork.first_block {
            anyhow::ensure!(
                self.proposal.parent == genesis.fork.first_parent,
                "bad parent of the first block of the fork"
            );
        }
        Ok(())
    }
}
