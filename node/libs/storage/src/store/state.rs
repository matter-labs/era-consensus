use anyhow::Context as _;
use zksync_consensus_roles::{attester, validator};

/// State of the `BlockStore`: continuous range of blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchStoreState {
    /// Stored block with the lowest number.
    /// If last is `None`, this is the first block that should be fetched.
    pub first: attester::BatchNumber,
    /// Stored block with the highest number.
    /// None iff store is empty.
    pub last: Option<attester::SyncBatch>,
}

impl Default for BatchStoreState {
    fn default() -> Self {
        Self {
            first: attester::BatchNumber(0),
            last: None,
        }
    }
}

impl BatchStoreState {
    /// Checks whether block with the given number is stored in the `BlockStore`.
    pub fn contains(&self, number: attester::BatchNumber) -> bool {
        let Some(last) = &self.last else { return false };
        self.first <= number && number <= last.number
    }

    /// Number of the next block that can be stored in the `BlockStore`.
    /// (i.e. `last` + 1).
    pub fn next(&self) -> attester::BatchNumber {
        match &self.last {
            Some(sb) => sb.number.next(),
            None => self.first,
        }
    }

    /// Verifies `BlockStoreState'.
    pub fn verify(&self) -> anyhow::Result<()> {
        if let Some(last) = &self.last {
            anyhow::ensure!(
                last.number >= self.first,
                "last block ({}) is before the first block ({})",
                last.number,
                self.first
            );
        }
        Ok(())
    }
}

/// State of the `BlockStore`: continuous range of blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockStoreState {
    /// Stored block with the lowest number.
    /// If last is `None`, this is the first block that should be fetched.
    pub first: validator::BlockNumber,
    /// Stored block with the highest number.
    /// None iff store is empty.
    pub last: Option<validator::CommitQC>,
}

impl Default for BlockStoreState {
    fn default() -> Self {
        Self {
            first: validator::BlockNumber(0),
            last: None,
        }
    }
}

impl BlockStoreState {
    /// Checks whether block with the given number is stored in the `BlockStore`.
    pub fn contains(&self, number: validator::BlockNumber) -> bool {
        let Some(last) = &self.last else { return false };
        self.first <= number && number <= last.header().number
    }

    /// Number of the next block that can be stored in the `BlockStore`.
    /// (i.e. `last` + 1).
    pub fn next(&self) -> validator::BlockNumber {
        match &self.last {
            Some(qc) => qc.header().number.next(),
            None => self.first,
        }
    }

    /// Verifies `BlockStoreState'.
    pub fn verify(&self, genesis: &validator::Genesis) -> anyhow::Result<()> {
        anyhow::ensure!(
            genesis.first_block <= self.first,
            "first block ({}) doesn't belong to the fork (which starts at block {})",
            self.first,
            genesis.first_block
        );
        if let Some(last) = &self.last {
            anyhow::ensure!(
                self.first <= last.header().number,
                "first block {} has bigger number than the last block {}",
                self.first,
                last.header().number
            );
            last.verify(genesis).context("last.verify()")?;
        }
        Ok(())
    }
}
