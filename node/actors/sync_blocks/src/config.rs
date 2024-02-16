//! Configuration for the `SyncBlocks` actor.
use zksync_concurrency::time;

/// Configuration for the `SyncBlocks` actor.
#[derive(Debug)]
pub struct Config {
    /// Maximum number of blocks to attempt to get concurrently from all peers in total.
    pub(crate) max_concurrent_blocks: usize,
    /// Maximum number of blocks to attempt to get concurrently from any single peer.
    pub(crate) max_concurrent_blocks_per_peer: usize,
    /// Interval between re-checking peers to get a specific block if no peers currently should have
    /// the block.
    pub(crate) sleep_interval_for_get_block: time::Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    /// Creates a new configuration with the provided mandatory params.
    pub fn new() -> Self {
        Self {
            max_concurrent_blocks: 20,
            max_concurrent_blocks_per_peer: 5,
            sleep_interval_for_get_block: time::Duration::seconds(10),
        }
    }

    /// Sets the maximum number of blocks to attempt to get concurrently.
    pub fn with_max_concurrent_blocks(mut self, blocks: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(blocks > 0, "Number of blocks must be positive");
        self.max_concurrent_blocks = blocks;
        Ok(self)
    }

    /// Maximum number of blocks to attempt to get concurrently from any single peer.
    pub fn with_max_concurrent_blocks_per_peer(mut self, blocks: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(blocks > 0, "Number of blocks must be positive");
        self.max_concurrent_blocks_per_peer = blocks;
        Ok(self)
    }

    /// Sets the interval between re-checking peers to get a specific block if no peers currently
    /// should have the block.
    pub fn with_sleep_interval_for_get_block(
        mut self,
        interval: time::Duration,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(interval.is_positive(), "Sleep interval must be positive");
        self.sleep_interval_for_get_block = interval;
        Ok(self)
    }
}
