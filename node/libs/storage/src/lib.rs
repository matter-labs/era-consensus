//! This module is responsible for persistent data storage, it provides schema-aware type-safe database access. Currently we use RocksDB,
//! but this crate only exposes an abstraction of a database, so we can easily switch to a different storage engine in the future.

use anyhow::Context as _;
use concurrency::{ctx, scope, sync::watch};
use roles::validator::{self, BlockNumber};
use std::{
    fmt, ops,
    path::Path,
    sync::{atomic::AtomicU64, RwLock},
};

mod block_store;
mod replica;
mod testonly;
#[cfg(test)]
mod tests;
mod types;

pub use crate::{
    block_store::{BlockStore, WriteBlockStore},
    replica::ReplicaStateStore,
    types::ReplicaState,
};

/// Main struct for the Storage module, it just contains the database. Provides a set of high-level
/// atomic operations on the database. It "contains" the following data:
///
/// - An append-only database of finalized blocks.
/// - A backup of the consensus replica state.
pub struct RocksdbStorage {
    /// Wrapped RocksDB instance. We don't need `RwLock` for synchronization *per se*, just to ensure
    /// that writes to the DB are linearized.
    inner: RwLock<rocksdb::DB>,
    /// In-memory cache for the last contiguous block number stored in the DB. The cache is used
    /// and updated by `Self::get_last_contiguous_block_number()`. Caching is based on the assumption
    /// that blocks are never removed from the DB.
    cached_last_contiguous_block_number: AtomicU64,
    /// Sender of numbers of written blocks.
    block_writes_sender: watch::Sender<BlockNumber>,
}

impl RocksdbStorage {
    /// Create a new Storage. It first tries to open an existing database, and if that fails it just creates a
    /// a new one. We need the genesis block of the chain as input.
    // TODO(bruno): we want to eventually start pruning old blocks, so having the genesis
    //   block might be unnecessary.
    pub async fn new(
        ctx: &ctx::Ctx,
        genesis_block: &validator::FinalBlock,
        path: &Path,
    ) -> anyhow::Result<Self> {
        scope::run!(ctx, |_, scope| async {
            Ok(scope
                .spawn_blocking(|| Self::new_blocking(genesis_block, path))
                .join(ctx)
                .await?)
        })
        .await
    }

    /// Blocking version of [`Self::new()`].
    fn new_blocking(genesis_block: &validator::FinalBlock, path: &Path) -> anyhow::Result<Self> {
        let mut options = rocksdb::Options::default();
        options.create_missing_column_families(true);
        options.create_if_missing(true);

        let db = rocksdb::DB::open(&options, path).context("Failed opening RocksDB")?;
        let this = Self {
            inner: RwLock::new(db),
            cached_last_contiguous_block_number: AtomicU64::new(0),
            block_writes_sender: watch::channel(genesis_block.block.number).0,
        };
        if let Some(stored_genesis_block) = this.block_blocking(genesis_block.block.number) {
            anyhow::ensure!(
                stored_genesis_block.block == genesis_block.block,
                "Mismatch between stored and expected genesis block"
            );
        } else {
            tracing::debug!(
                "Genesis block not present in RocksDB at `{path}`; saving {genesis_block:?}",
                path = path.display()
            );
            this.put_block_blocking(genesis_block);
        }
        Ok(this)
    }

    /// Acquires a read lock on the underlying DB.
    fn read(&self) -> impl ops::Deref<Target = rocksdb::DB> + '_ {
        self.inner.read().expect("DB lock is poisoned")
    }

    /// Acquires a write lock on the underlying DB.
    fn write(&self) -> impl ops::Deref<Target = rocksdb::DB> + '_ {
        self.inner.write().expect("DB lock is poisoned")
    }
}

impl fmt::Debug for RocksdbStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RocksdbStorage")
    }
}
