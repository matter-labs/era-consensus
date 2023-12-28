//! This module contains the methods to handle an append-only database of finalized blocks. Since we only store finalized blocks, this forms a
//! chain of blocks, not a tree (assuming we have all blocks and not have any gap). It allows for basic functionality like inserting a block,
//! getting a block, checking if a block is contained in the DB. We also store the head of the chain. Storing it explicitly allows us to fetch
//! the current head quickly.
use zksync_consensus_storage::{BlockStoreState, PersistentBlockStore, ReplicaState, ReplicaStore};
use anyhow::Context as _;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use std::sync::Arc;
use std::{fmt, path::Path, sync::RwLock};
use zksync_concurrency::{ctx, scope, error::Wrap as _};
use zksync_consensus_roles::validator;

/// Enum used to represent a key in the database. It also acts as a separator between different stores.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DatabaseKey {
    /// Key used to store the replica state.
    /// ReplicaState -> ReplicaState
    ReplicaState,
    /// Key used to store the finalized blocks.
    /// Block(validator::BlockNumber) -> validator::FinalBlock
    Block(validator::BlockNumber),
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
}

/// Main struct for the Storage module, it just contains the database. Provides a set of high-level
/// atomic operations on the database. It "contains" the following data:
///
/// - An append-only database of finalized blocks.
/// - A backup of the consensus replica state.
#[derive(Clone)]
pub(crate) struct RocksDB(Arc<RwLock<rocksdb::DB>>);

impl RocksDB {
    /// Create a new Storage. It first tries to open an existing database, and if that fails it just creates a
    /// a new one. We need the genesis block of the chain as input.
    pub(crate) async fn open(path: &Path) -> ctx::Result<Self> {
        let mut options = rocksdb::Options::default();
        options.create_missing_column_families(true);
        options.create_if_missing(true);
        Ok(Self(Arc::new(RwLock::new(
            scope::wait_blocking(|| {
                rocksdb::DB::open(&options, path).context("Failed opening RocksDB")
            }).await?,
        ))))
    }

    fn state_blocking(&self) -> anyhow::Result<Option<BlockStoreState>> {
        let db = self.0.read().unwrap();

        let mut options = ReadOptions::default();
        options.set_iterate_range(DatabaseKey::BLOCKS_START_KEY..);
        let Some(res) = db
            .iterator_opt(IteratorMode::Start, options)
            .next()
        else { return Ok(None) };
        let (_,first) = res.context("RocksDB error reading first stored block")?;
        let first: validator::FinalBlock =
            zksync_protobuf::decode(&first).context("Failed decoding first stored block bytes")?;
        
        let mut options = ReadOptions::default();
        options.set_iterate_range(DatabaseKey::BLOCKS_START_KEY..);
        let (_, last) = db
            .iterator_opt(DatabaseKey::BLOCK_HEAD_ITERATOR, options)
            .next()
            .context("last block not found")?
            .context("RocksDB error reading head block")?;
        let last: validator::FinalBlock =
            zksync_protobuf::decode(&last).context("Failed decoding head block bytes")?;

        Ok(Some(BlockStoreState {
            first: first.justification,
            last: last.justification,
        }))
    }
}

impl fmt::Debug for RocksDB {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RocksDB")
    }
}

#[async_trait::async_trait]
impl PersistentBlockStore for RocksDB {
    async fn state(&self, _ctx: &ctx::Ctx) -> ctx::Result<Option<BlockStoreState>> {
        Ok(scope::wait_blocking(|| self.state_blocking()).await?)
    }

    async fn block(
        &self,
        _ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<Option<validator::FinalBlock>> {
        scope::wait_blocking(|| {
            let db = self.0.read().unwrap();
            let Some(block) = db
                .get(DatabaseKey::Block(number).encode_key())
                .context("RocksDB error")?
            else { return Ok(None) };
            Ok(Some(zksync_protobuf::decode(&block).context("failed decoding block")?))
        }).await.wrap(number)
    }

    #[tracing::instrument(level = "debug", skip(self))]
    async fn store_next_block(
        &self,
        _ctx: &ctx::Ctx,
        block: &validator::FinalBlock,
    ) -> ctx::Result<()> {
        scope::wait_blocking(|| {
            let db = self.0.write().unwrap();
            let block_number = block.header().number;
            let mut write_batch = rocksdb::WriteBatch::default();
            write_batch.put(
                DatabaseKey::Block(block_number).encode_key(),
                zksync_protobuf::encode(block),
            );
            // Commit the transaction.
            db.write(write_batch)
                .context("Failed writing block to database")?;
            Ok(())
        }).await.wrap(block.header().number)
    }
}

#[async_trait::async_trait]
impl ReplicaStore for RocksDB {
    async fn state(&self, _ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>> {
        Ok(scope::wait_blocking(|| {
            let Some(raw_state) = self
                .0
                .read()
                .unwrap()
                .get(DatabaseKey::ReplicaState.encode_key())
                .context("Failed to get ReplicaState from RocksDB")?
            else {
                return Ok(None);
            };
            zksync_protobuf::decode(&raw_state)
                .map(Some)
                .context("Failed to decode replica state!")
        })
        .await?)
    }

    async fn set_state(&self, _ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()> {
        Ok(scope::wait_blocking(|| {
            self.0
                .write()
                .unwrap()
                .put(
                    DatabaseKey::ReplicaState.encode_key(),
                    zksync_protobuf::encode(state),
                )
                .context("Failed putting ReplicaState to RocksDB")
        })
        .await?)
    }
}
