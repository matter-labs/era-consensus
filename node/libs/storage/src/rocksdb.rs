//! This module contains the methods to handle an append-only database of finalized blocks. Since we only store finalized blocks, this forms a
//! chain of blocks, not a tree (assuming we have all blocks and not have any gap). It allows for basic functionality like inserting a block,
//! getting a block, checking if a block is contained in the DB. We also store the head of the chain. Storing it explicitly allows us to fetch
//! the current head quickly.
use crate::{
    traits::{BlockStore, ReplicaStateStore, WriteBlockStore},
    types::{MissingBlockNumbers, ReplicaState},
    StorageError, StorageResult,
};
use anyhow::Context as _;
use async_trait::async_trait;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use std::{
    fmt, ops,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        RwLock,
    },
};
use zksync_concurrency::{ctx, scope, sync::watch};
use zksync_consensus_roles::validator::{BlockNumber, FinalBlock};
use zksync_consensus_schema as schema;

/// Enum used to represent a key in the database. It also acts as a separator between different stores.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DatabaseKey {
    /// Key used to store the replica state.
    /// ReplicaState -> ReplicaState
    ReplicaState,
    /// Key used to store the finalized blocks.
    /// Block(BlockNumber) -> FinalBlock
    Block(BlockNumber),
}

impl DatabaseKey {
    /// Starting database key for blocks indexed by number. All other keys in the default column family
    /// are lower than this value.
    pub(crate) const BLOCKS_START_KEY: &'static [u8] = &u64::MIN.to_be_bytes();

    /// Iterator mode for the head block (i.e., a block with the greatest number).
    pub(crate) const BLOCK_HEAD_ITERATOR: IteratorMode<'static> =
        IteratorMode::From(&u64::MAX.to_be_bytes(), Direction::Reverse);

    /// Encodes this key for usage as a RocksDB key.
    ///
    /// # Implementation note
    ///
    /// This logic is maintainable only while the amount of non-block keys remains small.
    /// If more keys are added (especially if their number is not known statically), prefer using
    /// separate column families for them.
    pub(crate) fn encode_key(&self) -> Vec<u8> {
        match self {
            // Keys for non-block entries must be smaller than all block keys.
            Self::ReplicaState => vec![0],
            // Number encoding that monotonically increases with the number
            Self::Block(number) => number.0.to_be_bytes().to_vec(),
        }
    }

    /// Parses the specified bytes as a `Self::Block(_)` key.
    pub(crate) fn parse_block_key(raw_key: &[u8]) -> anyhow::Result<BlockNumber> {
        let raw_key = raw_key
            .try_into()
            .context("Invalid encoding for block key")?;
        Ok(BlockNumber(u64::from_be_bytes(raw_key)))
    }
}

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
        genesis_block: &FinalBlock,
        path: &Path,
    ) -> StorageResult<Self> {
        let mut options = rocksdb::Options::default();
        options.create_missing_column_families(true);
        options.create_if_missing(true);

        let db = scope::wait_blocking(|| {
            rocksdb::DB::open(&options, path)
                .context("Failed opening RocksDB")
                .map_err(StorageError::Database)
        })
        .await?;

        let this = Self {
            inner: RwLock::new(db),
            cached_last_contiguous_block_number: AtomicU64::new(0),
            block_writes_sender: watch::channel(genesis_block.header.number).0,
        };
        if let Some(stored_genesis_block) = this.block(ctx, genesis_block.header.number).await? {
            if stored_genesis_block.header != genesis_block.header {
                let err = anyhow::anyhow!("Mismatch between stored and expected genesis block");
                return Err(StorageError::Database(err));
            }
        } else {
            tracing::debug!(
                "Genesis block not present in RocksDB at `{path}`; saving {genesis_block:?}",
                path = path.display()
            );
            this.put_block(ctx, genesis_block).await?;
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

    fn head_block_blocking(&self) -> anyhow::Result<FinalBlock> {
        let db = self.read();

        let mut options = ReadOptions::default();
        options.set_iterate_range(DatabaseKey::BLOCKS_START_KEY..);
        let mut iter = db.iterator_opt(DatabaseKey::BLOCK_HEAD_ITERATOR, options);
        let (_, head_block) = iter
            .next()
            .context("Head block not found")?
            .context("RocksDB error reading head block")?;
        schema::decode(&head_block).context("Failed decoding head block bytes")
    }

    /// Returns a block with the least number stored in this database.
    fn first_block_blocking(&self) -> anyhow::Result<FinalBlock> {
        let db = self.read();

        let mut options = ReadOptions::default();
        options.set_iterate_range(DatabaseKey::BLOCKS_START_KEY..);
        let mut iter = db.iterator_opt(IteratorMode::Start, options);
        let (_, first_block) = iter
            .next()
            .context("First stored block not found")?
            .context("RocksDB error reading first stored block")?;
        schema::decode(&first_block).context("Failed decoding first stored block bytes")
    }

    fn last_contiguous_block_number_blocking(&self) -> anyhow::Result<BlockNumber> {
        let last_contiguous_block_number = self
            .cached_last_contiguous_block_number
            .load(Ordering::Relaxed);
        let last_contiguous_block_number = BlockNumber(last_contiguous_block_number);

        let last_contiguous_block_number =
            self.last_contiguous_block_number_impl(last_contiguous_block_number)?;

        // The cached value may have been updated by the other thread. Fortunately, we have a simple
        // protection against such "edit conflicts": the greater cached value is always valid and
        // should win.
        self.cached_last_contiguous_block_number
            .fetch_max(last_contiguous_block_number.0, Ordering::Relaxed);
        Ok(last_contiguous_block_number)
    }

    // Implementation that is not aware of caching specifics. The only requirement for the method correctness
    // is for the `cached_last_contiguous_block_number` to be present in the database.
    fn last_contiguous_block_number_impl(
        &self,
        cached_last_contiguous_block_number: BlockNumber,
    ) -> anyhow::Result<BlockNumber> {
        let db = self.read();

        let mut options = ReadOptions::default();
        let start_key = DatabaseKey::Block(cached_last_contiguous_block_number).encode_key();
        options.set_iterate_range(start_key..);
        let iter = db.iterator_opt(IteratorMode::Start, options);
        let iter = iter
            .map(|bytes| {
                let (key, _) = bytes.context("RocksDB error iterating over block numbers")?;
                DatabaseKey::parse_block_key(&key)
            })
            .fuse();

        let mut prev_block_number = cached_last_contiguous_block_number;
        for block_number in iter {
            let block_number = block_number?;
            if block_number > prev_block_number.next() {
                return Ok(prev_block_number);
            }
            prev_block_number = block_number;
        }
        Ok(prev_block_number)
    }

    /// Gets a block by its number.
    fn block_blocking(&self, number: BlockNumber) -> anyhow::Result<Option<FinalBlock>> {
        let db = self.read();

        let Some(raw_block) = db
            .get(DatabaseKey::Block(number).encode_key())
            .with_context(|| format!("RocksDB error reading block #{number}"))?
        else {
            return Ok(None);
        };
        let block = schema::decode(&raw_block)
            .with_context(|| format!("Failed decoding block #{number}"))?;
        Ok(Some(block))
    }

    /// Iterates over block numbers in the specified `range` that the DB *does not* have.
    fn missing_block_numbers_blocking(
        &self,
        range: ops::Range<BlockNumber>,
    ) -> anyhow::Result<Vec<BlockNumber>> {
        let db = self.read();

        let mut options = ReadOptions::default();
        let start_key = DatabaseKey::Block(range.start).encode_key();
        let end_key = DatabaseKey::Block(range.end).encode_key();
        options.set_iterate_range(start_key..end_key);

        let iter = db.iterator_opt(IteratorMode::Start, options);
        let iter = iter
            .map(|bytes| {
                let (key, _) = bytes.context("RocksDB error iterating over block numbers")?;
                DatabaseKey::parse_block_key(&key)
            })
            .fuse();

        MissingBlockNumbers::new(range, iter).collect()
    }

    // ---------------- Write methods ----------------

    /// Insert a new block into the database.
    fn put_block_blocking(&self, finalized_block: &FinalBlock) -> anyhow::Result<()> {
        let db = self.write();
        let block_number = finalized_block.header.number;
        tracing::debug!("Inserting new block #{block_number} into the database.");

        let mut write_batch = rocksdb::WriteBatch::default();
        write_batch.put(
            DatabaseKey::Block(block_number).encode_key(),
            schema::encode(finalized_block),
        );
        // Commit the transaction.
        db.write(write_batch)
            .context("Failed writing block to database")?;
        drop(db);

        self.block_writes_sender.send_replace(block_number);
        Ok(())
    }

    fn replica_state_blocking(&self) -> anyhow::Result<Option<ReplicaState>> {
        let Some(raw_state) = self
            .read()
            .get(DatabaseKey::ReplicaState.encode_key())
            .context("Failed to get ReplicaState from RocksDB")?
        else {
            return Ok(None);
        };
        schema::decode(&raw_state)
            .map(Some)
            .context("Failed to decode replica state!")
    }

    fn put_replica_state_blocking(&self, replica_state: &ReplicaState) -> anyhow::Result<()> {
        self.write()
            .put(
                DatabaseKey::ReplicaState.encode_key(),
                schema::encode(replica_state),
            )
            .context("Failed putting ReplicaState to RocksDB")
    }
}

impl fmt::Debug for RocksdbStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RocksdbStorage")
    }
}

#[async_trait]
impl BlockStore for RocksdbStorage {
    async fn head_block(&self, _ctx: &ctx::Ctx) -> StorageResult<FinalBlock> {
        scope::wait_blocking(|| self.head_block_blocking().map_err(StorageError::Database)).await
    }

    async fn first_block(&self, _ctx: &ctx::Ctx) -> StorageResult<FinalBlock> {
        scope::wait_blocking(|| self.first_block_blocking().map_err(StorageError::Database)).await
    }

    async fn last_contiguous_block_number(&self, _ctx: &ctx::Ctx) -> StorageResult<BlockNumber> {
        scope::wait_blocking(|| {
            self.last_contiguous_block_number_blocking()
                .map_err(StorageError::Database)
        })
        .await
    }

    async fn block(
        &self,
        _ctx: &ctx::Ctx,
        number: BlockNumber,
    ) -> StorageResult<Option<FinalBlock>> {
        scope::wait_blocking(|| self.block_blocking(number).map_err(StorageError::Database)).await
    }

    async fn missing_block_numbers(
        &self,
        _ctx: &ctx::Ctx,
        range: ops::Range<BlockNumber>,
    ) -> StorageResult<Vec<BlockNumber>> {
        scope::wait_blocking(|| {
            self.missing_block_numbers_blocking(range)
                .map_err(StorageError::Database)
        })
        .await
    }

    fn subscribe_to_block_writes(&self) -> watch::Receiver<BlockNumber> {
        self.block_writes_sender.subscribe()
    }
}

#[async_trait]
impl WriteBlockStore for RocksdbStorage {
    async fn put_block(&self, _ctx: &ctx::Ctx, block: &FinalBlock) -> StorageResult<()> {
        scope::wait_blocking(|| {
            self.put_block_blocking(block)
                .map_err(StorageError::Database)
        })
        .await
    }
}

#[async_trait]
impl ReplicaStateStore for RocksdbStorage {
    async fn replica_state(&self, _ctx: &ctx::Ctx) -> StorageResult<Option<ReplicaState>> {
        scope::wait_blocking(|| {
            self.replica_state_blocking()
                .map_err(StorageError::Database)
        })
        .await
    }

    async fn put_replica_state(
        &self,
        _ctx: &ctx::Ctx,
        replica_state: &ReplicaState,
    ) -> StorageResult<()> {
        scope::wait_blocking(|| {
            self.put_replica_state_blocking(replica_state)
                .map_err(StorageError::Database)
        })
        .await
    }
}
