use std::collections::VecDeque;

use zksync_consensus_roles::validator;

#[derive(Debug)]
pub(crate) struct BlockStore {
    pub(crate) queued: BlockStoreState,
    pub(crate) persisted: BlockStoreState,
    pub(crate) cache: VecDeque<validator::Block>,
}

impl BlockStore {
    /// Minimal number of most recent blocks to keep in memory.
    /// It allows to serve the recent blocks to peers fast, even
    /// if persistent storage reads are slow (like in RocksDB).
    /// `BlockStore` may keep in memory more blocks in case
    /// blocks are queued faster than they are persisted.
    pub(crate) const CACHE_CAPACITY: usize = 100;

    pub(crate) fn block(&self, n: validator::BlockNumber) -> Option<validator::Block> {
        let first = self.cache.front()?;
        self.cache
            .get(n.0.checked_sub(first.number().0)? as usize)
            .cloned()
    }

    /// Tries to push the next block to cache.
    /// Noop if provided block is not the expected one.
    /// Returns true iff cache has been modified.
    pub(crate) fn try_push(&mut self, block: validator::Block) -> bool {
        if self.queued.next() != block.number() {
            return false;
        }
        self.queued.last = Some(Last::from(&block));
        self.cache.push_back(block);
        self.truncate_cache();
        true
    }

    /// Updates `persisted` field.
    #[tracing::instrument(skip_all)]
    pub(crate) fn update_persisted(&mut self, persisted: BlockStoreState) -> anyhow::Result<()> {
        if persisted.next() < self.persisted.next() {
            anyhow::bail!("head block has been removed from storage, this is not supported");
        }
        self.persisted = persisted;
        if self.queued.first < self.persisted.first {
            self.queued.first = self.persisted.first;
        }
        // If persisted blocks overtook the queue (blocks were fetched via some side-channel),
        // it means we need to reset the cache - otherwise we would have a gap.
        if self.queued.next() < self.persisted.next() {
            self.queued = self.persisted.clone();
            self.cache.clear();
        }
        self.truncate_cache();
        Ok(())
    }

    /// If cache size has been exceeded, remove entries which were already persisted.
    pub(crate) fn truncate_cache(&mut self) {
        while self.cache.len() > Self::CACHE_CAPACITY
            && self.persisted.next() > self.cache[0].number()
        {
            self.cache.pop_front();
        }
    }
}

/// State of the `BlockStore`: continuous range of blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockStoreState {
    /// Stored block with the lowest number.
    /// If `last` is `None`, this is the first block that should be fetched.
    pub first: validator::BlockNumber,
    /// Stored block with the highest number.
    /// If it is lower than genesis.first, then it will be set to `Last::Number` and
    /// the first blocks to be fetched will need to include the non-consensus justification
    /// (see `PreGenesisBlock`).
    /// None iff store is empty.
    pub last: Option<Last>,
}

impl BlockStoreState {
    /// Checks whether block with the given number is stored in the `BlockStore`.
    pub fn contains(&self, number: validator::BlockNumber) -> bool {
        let Some(last) = &self.last else { return false };
        self.first <= number && number <= last.number()
    }

    /// Returns the number of the head of the chain. We might not have the block head in the store.
    pub fn head(&self) -> validator::BlockNumber {
        match &self.last {
            Some(last) => last.number(),
            None => self.first.prev().unwrap_or(validator::BlockNumber(0)),
        }
    }
    /// Number of the next block that can be stored in the `BlockStore`.
    /// (i.e. `last` + 1).
    pub fn next(&self) -> validator::BlockNumber {
        match &self.last {
            Some(last) => last.number().next(),
            None => self.first,
        }
    }

    /// Verifies `BlockStoreState'.
    pub fn verify(&self) -> anyhow::Result<()> {
        if let Some(last) = &self.last {
            anyhow::ensure!(
                self.first <= last.number(),
                "first block {} has bigger number than the last block {}",
                self.first,
                last.number(),
            );
        }
        Ok(())
    }
}

/// Last block in the block store (see `BlockStoreState`).
/// Note that the commit qc is required in the case the block
/// has been finalized by the consensus.
#[derive(Debug, Clone, PartialEq)]
pub enum Last {
    /// `<genesis.first_block`.
    PreGenesis(validator::BlockNumber),
    /// `>=genesis.first_block`.
    FinalV2(validator::v2::CommitQC),
}

impl From<&validator::Block> for Last {
    fn from(b: &validator::Block) -> Last {
        use validator::Block as B;
        match b {
            B::PreGenesis(b) => Last::PreGenesis(b.number),
            B::FinalV2(b) => Last::FinalV2(b.justification.clone()),
        }
    }
}

impl Last {
    /// Converts Last to block number.
    pub fn number(&self) -> validator::BlockNumber {
        match self {
            Last::PreGenesis(n) => *n,
            Last::FinalV2(qc) => qc.header().number,
        }
    }
}
