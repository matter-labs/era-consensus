//! RocksDB-based implementation of PersistentBlockStore and ReplicaStore.
use anyhow::Context as _;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use std::{
    fmt,
    path::Path,
    sync::{Arc, RwLock},
};
use zksync_concurrency::{ctx, error::Wrap as _, scope, sync};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::{
    BlockStoreState, Last, PersistentBlockStore, ReplicaState, ReplicaStore,
};

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

struct Inner {
    genesis: validator::Genesis,
    persisted: sync::watch::Sender<BlockStoreState>,
    // Surround RocksDB with a read-write lock not because we need write access to its API,
    // but rather to be able to simulate transactions where only one thread will modify the DB,
    // after doing some consistency checks against other in-memory data structures.
    db: RwLock<rocksdb::DB>,
}

/// Main struct for the Storage module, it just contains the database. Provides a set of high-level
/// atomic operations on the database. It "contains" the following data:
///
/// - An append-only database of finalized blocks.
/// - A backup of the consensus replica state.
#[derive(Clone)]
pub(crate) struct RocksDB(Arc<Inner>);

impl RocksDB {
    /// Create a new Storage. It first tries to open an existing database, and if that fails it just creates a
    /// a new one. We need the genesis block of the chain as input.
    pub(crate) async fn open(genesis: validator::Genesis, path: &Path) -> ctx::Result<Self> {
        let mut options = rocksdb::Options::default();
        options.create_missing_column_families(true);
        options.create_if_missing(true);
        let db = scope::wait_blocking(|| {
            rocksdb::DB::open(&options, path).context("Failed opening RocksDB")
        })
        .await?;

        Ok(Self(Arc::new(Inner {
            persisted: sync::watch::channel(BlockStoreState {
                // `RocksDB` is assumed to store all blocks starting from genesis.
                first: genesis.first_block,
                last: scope::wait_blocking(|| Self::last_blocking(&db)).await?,
            })
            .0,
            genesis,
            db: RwLock::new(db),
        })))
    }

    fn last_blocking(db: &rocksdb::DB) -> anyhow::Result<Option<Last>> {
        let mut options = ReadOptions::default();
        options.set_iterate_range(DatabaseKey::BLOCKS_START_KEY..);
        let Some(res) = db
            .iterator_opt(DatabaseKey::BLOCK_HEAD_ITERATOR, options)
            .next()
        else {
            return Ok(None);
        };
        let (_, raw) = res.context("RocksDB error reading head block")?;
        let b: validator::Block =
            zksync_protobuf::decode(&raw).context("Failed decoding head block bytes")?;
        Ok(Some((&b).into()))
    }
}

impl fmt::Debug for RocksDB {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RocksDB")
    }
}

#[async_trait::async_trait]
impl PersistentBlockStore for RocksDB {
    async fn genesis(&self, _ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis> {
        Ok(self.0.genesis.clone())
    }

    fn persisted(&self) -> sync::watch::Receiver<BlockStoreState> {
        self.0.persisted.subscribe()
    }

    async fn block(
        &self,
        _ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Block> {
        scope::wait_blocking(|| {
            let db = self.0.db.read().unwrap();
            let block = db
                .get(DatabaseKey::Block(number).encode_key())
                .context("RocksDB error")?
                .context("not found")?;
            Ok(zksync_protobuf::decode(&block).context("failed decoding block")?)
        })
        .await
        .wrap(number)
    }

    async fn verify_pregenesis_block(
        &self,
        _ctx: &ctx::Ctx,
        _block: &validator::PreGenesisBlock,
    ) -> ctx::Result<()> {
        Err(anyhow::format_err!("pre-genesis blocks not supported").into())
    }

    #[tracing::instrument(skip_all, fields(l2_block = %block.number()))]
    async fn queue_next_block(&self, _ctx: &ctx::Ctx, block: validator::Block) -> ctx::Result<()> {
        scope::wait_blocking(|| {
            // We use an exclusive lock so no other thread can change the expected block number
            // between the check and the insertion into the database. We could use a RocksDB
            // transaction instead, but this is probably cheaper.
            #[allow(clippy::readonly_write_lock)]
            let db = self.0.db.write().unwrap();
            let want = self.0.persisted.borrow().next();
            anyhow::ensure!(
                block.number() == want,
                "got {:?} want {want:?}",
                block.number()
            );
            let mut write_batch = rocksdb::WriteBatch::default();
            write_batch.put(
                DatabaseKey::Block(block.number()).encode_key(),
                zksync_protobuf::encode(&block),
            );
            // Commit the transaction.
            db.write(write_batch)
                .context("Failed writing block to database")?;
            self.0
                .persisted
                .send_modify(|p| p.last = Some((&block).into()));
            Ok(())
        })
        .await
        .context(block.number())?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl ReplicaStore for RocksDB {
    async fn state(&self, _ctx: &ctx::Ctx) -> ctx::Result<ReplicaState> {
        Ok(scope::wait_blocking(|| {
            let Some(raw_state) = self
                .0
                .db
                .read()
                .unwrap()
                .get(DatabaseKey::ReplicaState.encode_key())
                .context("Failed to get ReplicaState from RocksDB")?
            else {
                return Ok(ReplicaState::default());
            };
            zksync_protobuf::decode(&raw_state).context("Failed to decode replica state!")
        })
        .await?)
    }

    async fn set_state(&self, _ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()> {
        Ok(scope::wait_blocking(|| {
            self.0
                .db
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
