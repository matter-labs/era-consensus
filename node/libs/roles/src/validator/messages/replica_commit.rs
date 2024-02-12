use super::*;

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
    pub fn verify(&self, genesis: &Genesis, allow_past_forks: bool) -> anyhow::Result<()> {
        if !allow_past_forks {
            anyhow::ensure!(self.view.fork == genesis.forks.current());
        }
        // Currently we process CommitQCs from past forks when verifying FinalBlocks
        // during synchronization. Eventually we will switch to synchronization without
        // CommitQCs for blocks from the past forks, and then we can always enforce
        // `genesis.forks.current()` instead.
        let want = genesis.forks.find(self.proposal.number);
        anyhow::ensure!(
            want == Some(self.view.fork),
            "bad fork: got {:?} want {want:?} for block {}",
            self.view.fork,
            self.proposal.number
        );
        Ok(())
    }
}
