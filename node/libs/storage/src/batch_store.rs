//! Defines storage layer for batches of blocks.
use anyhow::Context as _;
use std::{collections::VecDeque, fmt, sync::Arc};
use zksync_concurrency::{ctx, scope, sync};
use zksync_consensus_roles::{attester, validator};

/// State of the `BatchStore`: continuous range of batches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchStoreState {
    /// Stored batch with the lowest number.
    /// If last is `None`, this is the first batch that should be fetched.
    pub first: attester::BatchNumber,
    /// The last stored L1 batch.
    /// None iff store is empty.
    pub last: Option<attester::SyncBatch>,
}

impl BatchStoreState {
    /// Checks whether batch with the given number is stored in the `BatchStore`.
    pub fn contains(&self, number: attester::BatchNumber) -> bool {
        let Some(last) = &self.last else { return false };
        self.first <= number && number <= last.number
    }

    /// Number of the next batch that can be stored in the `BatchStore`.
    /// (i.e. `last` + 1).
    pub fn next(&self) -> attester::BatchNumber {
        match &self.last {
            Some(last) => last.number.next(),
            None => self.first,
        }
    }

    /// Verifies `BatchStoreState'.
    pub fn verify(&self) -> anyhow::Result<()> {
        if let Some(last) = &self.last {
            anyhow::ensure!(
                self.first <= last.clone().number,
                "first batch {} has bigger number than the last batch {}",
                self.first,
                last.number
            );
        }
        Ok(())
    }
}

/// Trait for the shared state of batches between the consensus and the execution layer.
#[async_trait::async_trait]
pub trait PersistentBatchStore: 'static + fmt::Debug + Send + Sync {
    /// Range of batches persisted in storage.
    fn persisted(&self) -> sync::watch::Receiver<BatchStoreState>;

    /// Get the L1 batch from storage with the highest number.
    ///
    /// Returns `None` if no batches have been created yet.
    async fn last_batch(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<attester::BatchNumber>>;

    /// Get the L1 batch QC from storage with the highest number.
    ///
    /// Returns `None` if we don't have a QC for any of the batches yet.
    async fn last_batch_qc(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<attester::BatchQC>>;

    /// Returns the batch with the given number.
    async fn get_batch(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::SyncBatch>>;

    /// Returns the QC of the batch with the given number.
    async fn get_batch_qc(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::BatchQC>>;

    /// Store the given batch QC in the storage.
    async fn store_qc(&self, ctx: &ctx::Ctx, qc: attester::BatchQC) -> ctx::Result<()>;

    /// Queue the batch to be persisted in storage.
    /// `queue_next_batch()` may return BEFORE the batch is actually persisted,
    /// but if the call succeeded the batch is expected to be persisted eventually.
    /// Implementations are only required to accept a batch directly after the previous queued
    /// batch, starting with `persisted().borrow().next()`.
    async fn queue_next_batch(&self, ctx: &ctx::Ctx, batch: attester::SyncBatch)
        -> ctx::Result<()>;
}

/// Inner state of the `BatchStore`.
#[derive(Debug)]
struct Inner {
    /// Batches that are queued to be persisted.
    queued: BatchStoreState,
    /// Batches that are already persisted.
    persisted: BatchStoreState,
    cache: VecDeque<attester::SyncBatch>,
}

impl Inner {
    /// Minimal number of most recent batches to keep in memory.
    /// It allows to serve the recent batches to peers fast, even
    /// if persistent storage reads are slow (like in RocksDB).
    /// `BatchStore` may keep in memory more batches in case
    /// batches are queued faster than they are persisted.
    const CACHE_CAPACITY: usize = 10;

    /// Tries to push the next batch to cache.
    /// Noop if provided batch is not the expected one.
    /// Returns true iff cache has been modified.
    fn try_push(&mut self, batch: attester::SyncBatch) -> bool {
        if self.queued.next() != batch.number {
            return false;
        }
        self.queued.last = Some(batch.clone());
        self.cache.push_back(batch.clone());
        self.truncate_cache();
        true
    }

    fn truncate_cache(&mut self) {
        while self.cache.len() > Self::CACHE_CAPACITY
            && self.persisted.contains(self.cache[0].number)
        {
            self.cache.pop_front();
        }
    }

    fn batch(&self, n: attester::BatchNumber) -> Option<attester::SyncBatch> {
        // Subtraction is safe, because batches in cache are
        // stored in increasing order of batch number.
        let first = self.cache.front()?;
        self.cache.get((n.0 - first.number.0) as usize).cloned()
    }
}

/// A wrapper around a PersistentBatchStore.
#[derive(Debug)]
pub struct BatchStore {
    inner: sync::watch::Sender<Inner>,
    persistent: Box<dyn PersistentBatchStore>,
}

/// Runner of the BatchStore background tasks.
#[must_use]
#[derive(Debug, Clone)]
pub struct BatchStoreRunner(Arc<BatchStore>);

impl BatchStoreRunner {
    /// Runs the background tasks of the BatchStore.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let res = scope::run!(ctx, |ctx, s| async {
            let persisted = self.0.persistent.persisted();
            let mut queue_next = persisted.borrow().next();
            // Task truncating cache whenever a batch gets persisted.
            s.spawn::<()>(async {
                let mut persisted = persisted;
                loop {
                    let persisted = sync::changed(ctx, &mut persisted).await?.clone();
                    self.0.inner.send_modify(|inner| {
                        inner.persisted = persisted;
                        inner.truncate_cache();
                    });
                }
            });
            let inner = &mut self.0.inner.subscribe();
            loop {
                let batch = sync::wait_for(ctx, inner, |inner| inner.queued.contains(queue_next))
                    .await?
                    .batch(queue_next)
                    .unwrap();

                queue_next = queue_next.next();

                self.0.persistent.queue_next_batch(ctx, batch).await?;
            }
        })
        .await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}

impl BatchStore {
    /// Constructs a BatchStore.
    /// BatchStore takes ownership of the passed PersistentBatchStore,
    /// i.e. caller should modify the underlying persistent storage
    /// ONLY through the constructed BatchStore.
    pub async fn new(
        _ctx: &ctx::Ctx,
        persistent: Box<dyn PersistentBatchStore>,
    ) -> ctx::Result<(Arc<Self>, BatchStoreRunner)> {
        let persisted = persistent.persisted().borrow().clone();
        persisted.verify().context("state.verify()")?;
        let this = Arc::new(Self {
            inner: sync::watch::channel(Inner {
                queued: persisted.clone(),
                persisted,
                cache: VecDeque::new(),
            })
            .0,
            persistent,
        });
        Ok((this.clone(), BatchStoreRunner(this)))
    }

    /// Available batches (in memory & persisted).
    pub fn queued(&self) -> BatchStoreState {
        self.inner.borrow().queued.clone()
    }

    /// Fetches a batch (from queue or persistent storage).
    pub async fn batch(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::SyncBatch>> {
        {
            let inner = self.inner.borrow();
            if !inner.queued.contains(number) {
                return Ok(None);
            }
            if let Some(batch) = inner.batch(number) {
                return Ok(Some(batch));
            }
        }
        let batch = self
            .persistent
            .get_batch(ctx, number)
            .await
            .context("persistent.batch()")?;
        Ok(batch)
    }

    /// Append batch to a queue to be persisted eventually.
    /// Since persisting a batch may take a significant amount of time,
    /// BatchStore contains a queue of batches waiting to be persisted.
    /// `queue_batch()` adds a batch to the queue as soon as all intermediate
    /// Batches are queued_state as well. Queue is unbounded, so it is caller's
    /// responsibility to manage the queue size.
    pub async fn queue_batch(
        &self,
        ctx: &ctx::Ctx,
        batch: attester::SyncBatch,
        _genesis: validator::Genesis,
    ) -> ctx::Result<()> {
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            inner.queued.next() >= batch.number
        })
        .await?;

        self.inner
            .send_if_modified(|inner| inner.try_push(batch.clone()));

        Ok(())
    }

    /// Wait until the database has a batch, then attach the corresponding QC.
    pub async fn queue_batch_qc(&self, ctx: &ctx::Ctx, qc: attester::BatchQC) -> ctx::Result<()> {
        // The `store_qc` implementation in `zksync-era` retries the insertion of the QC if the payload
        // isn't yet available, but to be safe we can wait for the availability signal here as well.
        sync::wait_for(ctx, &mut self.persistent.persisted(), |persisted| {
            qc.message.number < persisted.next()
        })
        .await?;
        // Now it's definitely safe to store it.
        self.persistent.store_qc(ctx, qc).await?;
        Ok(())
    }

    /// Waits until the given batch is queued (in memory, or persisted).
    /// Note that it doesn't mean that the batch is actually available, as old batches might get pruned.
    pub async fn wait_until_queued(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::OrCanceled<BatchStoreState> {
        Ok(sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            number < inner.queued.next()
        })
        .await?
        .queued
        .clone())
    }

    /// Waits until the given batch is stored persistently.
    /// Note that it doesn't mean that the batch is actually available, as old batches might get pruned.
    pub async fn wait_until_persisted(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::OrCanceled<BatchStoreState> {
        Ok(
            sync::wait_for(ctx, &mut self.persistent.persisted(), |persisted| {
                number < persisted.next()
            })
            .await?
            .clone(),
        )
    }
}
