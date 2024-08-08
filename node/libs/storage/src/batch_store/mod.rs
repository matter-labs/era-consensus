//! Defines storage layer for batches of blocks.
use anyhow::Context as _;
use std::{collections::VecDeque, fmt, sync::Arc};
use tracing::Instrument;
use zksync_concurrency::{ctx, error::Wrap as _, scope, sync};
use zksync_consensus_roles::{attester, validator};

mod metrics;

/// State of the `BatchStore`: continuous range of batches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchStoreState {
    /// Stored batch with the lowest number.
    /// If last is `None`, this is the first batch that should be fetched.
    pub first: attester::BatchNumber,
    /// The last stored L1 batch.
    /// None iff store is empty.
    pub last: Option<attester::BatchNumber>,
}

impl BatchStoreState {
    /// Checks whether batch with the given number is stored in the `BatchStore`.
    pub fn contains(&self, number: attester::BatchNumber) -> bool {
        let Some(last) = self.last else { return false };
        self.first <= number && number <= last
    }

    /// Number of the next batch that can be stored in the `BatchStore`.
    /// (i.e. `last` + 1).
    pub fn next(&self) -> attester::BatchNumber {
        match &self.last {
            Some(last) => last.next(),
            None => self.first,
        }
    }

    /// Verifies `BatchStoreState'.
    pub fn verify(&self) -> anyhow::Result<()> {
        if let Some(last) = self.last {
            anyhow::ensure!(
                self.first <= last,
                "first batch {} has bigger number than the last batch {}",
                self.first,
                last
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

    /// Get the next L1 batch for which attesters are expected to produce a quorum certificate.
    ///
    /// An external node might never have a complete history of L1 batch QCs. Once the L1 batch is included on L1,
    /// the external nodes might use the [attester::SyncBatch] route to obtain them, in which case they will not
    /// have a QC and no reason to get them either. The main node, however, will want to have a QC for all batches.
    async fn next_batch_to_attest(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<attester::BatchNumber>>;

    /// Get the L1 batch QC from storage with the highest number.
    ///
    /// Returns `None` if we don't have a QC for any of the batches yet.
    async fn last_batch_qc(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<attester::BatchQC>>;

    /// Returns the [attester::SyncBatch] with the given number, which is used by peers
    /// to catch up with L1 batches that they might have missed if they went offline.
    async fn get_batch(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::SyncBatch>>;

    /// Returns the [attester::Batch] with the given number, which is the `message` that
    /// appears in [attester::BatchQC], and represents the content that needs to be signed
    /// by the attesters.
    async fn get_batch_to_sign(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::Batch>>;

    /// Returns the QC of the batch with the given number.
    async fn get_batch_qc(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::BatchQC>>;

    /// Store the given batch QC in the storage persistently.
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
    ///
    /// This reflects the state of the `cache`. Its source is mainly the gossip layer (the RPC protocols started in `Network::run_stream`):
    /// * the node pushes `SyncBatch` records which appear in `queued` to its gossip peers
    /// * the node pulls `SyncBatch` records that it needs from gossip peers that reported to have them, and adds them to `queued`
    /// * the `BatchStoreRunner` looks for new items in `queued` and pushes them into the `PersistentBatchStore`
    ///
    /// XXX: There doesn't seem to be anything that currently actively pushes into `queued` from outside gossip,
    /// like it happens with the `BlockStore::queue_block` being called from BFT.
    queued: BatchStoreState,
    /// Batches that are already persisted.
    ///
    /// This reflects the state of the database. Its source is mainly the `PersistentBatchStore`:
    /// * the `BatchStoreRunner` subscribes to `PersistedBatchStore::persisted()` and copies its contents to here;
    /// * it also uses the opportunity to clear items from the `cache`
    /// * but notably doesn't update `queued`, which would cause the data to be gossiped
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
        self.queued.last = Some(batch.number);
        self.cache.push_back(batch.clone());
        self.truncate_cache();
        true
    }

    #[tracing::instrument(skip_all)]
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
        #[vise::register]
        static COLLECTOR: vise::Collector<Option<metrics::BatchStoreState>> =
            vise::Collector::new();
        let store_ref = Arc::downgrade(&self.0);
        let _ = COLLECTOR.before_scrape(move || Some(store_ref.upgrade()?.scrape_metrics()));

        let res = scope::run!(ctx, |ctx, s| async {
            let persisted = self.0.persistent.persisted();
            let mut queue_next = persisted.borrow().next();
            // Task truncating cache whenever a batch gets persisted.
            s.spawn::<()>(async {
                let mut persisted = persisted;
                loop {
                    async {
                        let persisted = sync::changed(ctx, &mut persisted)
                            .instrument(tracing::info_span!("wait_for_batch_store_change"))
                            .await?
                            .clone();
                        self.0.inner.send_modify(|inner| {
                            // XXX: In `BlockStoreRunner` update both the `queued` and the `persisted` here.
                            inner.persisted = persisted;
                            inner.truncate_cache();
                        });

                        ctx::Ok(())
                    }
                    .instrument(tracing::info_span!("truncate_batch_cache_iter"))
                    .await?;
                }
            });
            let inner = &mut self.0.inner.subscribe();
            loop {
                async {
                    let batch =
                        sync::wait_for(ctx, inner, |inner| inner.queued.contains(queue_next))
                            .instrument(tracing::info_span!("wait_for_next_batch"))
                            .await?
                            .batch(queue_next)
                            .unwrap();

                    queue_next = queue_next.next();

                    self.0.persistent.queue_next_batch(ctx, batch).await?;

                    ctx::Ok(())
                }
                .instrument(tracing::info_span!("queue_persist_batch_iter"))
                .await?;
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
        let t = metrics::PERSISTENT_BATCH_STORE.batch_latency.start();

        let batch = self
            .persistent
            .get_batch(ctx, number)
            .await
            .wrap("persistent.get_batch()")?;

        t.observe();
        Ok(batch)
    }

    /// Retrieve the next batch number that doesn't have a QC yet and will need to be signed.
    pub async fn next_batch_to_attest(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<attester::BatchNumber>> {
        let t = metrics::PERSISTENT_BATCH_STORE
            .next_batch_to_attest_latency
            .start();

        let batch = self
            .persistent
            .next_batch_to_attest(ctx)
            .await
            .wrap("persistent.next_batch_to_attest()")?;

        t.observe();
        Ok(batch)
    }

    /// Retrieve a batch to be signed.
    ///
    /// This might be `None` even if the L1 batch already exists, because the commitment
    /// in it is populated asynchronously.
    pub async fn batch_to_sign(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::Batch>> {
        let t = metrics::PERSISTENT_BATCH_STORE
            .batch_to_sign_latency
            .start();

        let batch = self
            .persistent
            .get_batch_to_sign(ctx, number)
            .await
            .wrap("persistent.get_batch_to_sign()")?;

        t.observe();
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
        let t = metrics::BATCH_STORE.queue_batch.start();

        // XXX: Once we can validate `SyncBatch::proof` we should do it before adding the
        // batch to the cache, otherwise a malicious peer could serve us data that prevents
        // other inputs from entering the queue. It will also cause it to be gossiped at the moment.
        sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            inner.queued.next() >= batch.number
        })
        .await?;

        self.inner
            .send_if_modified(|inner| inner.try_push(batch.clone()));

        t.observe();
        Ok(())
    }

    /// Wait until the database has a batch, then attach the corresponding QC.
    #[tracing::instrument(skip_all, fields(l1_batch = %qc.message.number))]
    pub async fn persist_batch_qc(&self, ctx: &ctx::Ctx, qc: attester::BatchQC) -> ctx::Result<()> {
        let t = metrics::BATCH_STORE.persist_batch_qc.start();
        // The `store_qc` implementation in `zksync-era` retries the insertion of the QC if the payload
        // isn't yet available, but to be safe we can wait for the availability signal here as well.
        sync::wait_for(ctx, &mut self.persistent.persisted(), |persisted| {
            qc.message.number < persisted.next()
        })
        .await?;
        // Now it's definitely safe to store it.
        metrics::BATCH_STORE
            .last_persisted_batch_qc
            .set(qc.message.number.0);

        self.persistent.store_qc(ctx, qc).await?;

        t.observe();
        Ok(())
    }

    /// Waits until the given batch is queued (in memory, or persisted).
    /// Note that it doesn't mean that the batch is actually available, as old batches might get pruned.
    #[tracing::instrument(skip_all, fields(l1_batch = %number))]
    pub async fn wait_until_queued(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::OrCanceled<BatchStoreState> {
        let t = metrics::BATCH_STORE.wait_until_queued.start();

        let state = sync::wait_for(ctx, &mut self.inner.subscribe(), |inner| {
            number < inner.queued.next()
        })
        .await?
        .queued
        .clone();

        t.observe();
        Ok(state)
    }

    /// Waits until the given batch is stored persistently.
    /// Note that it doesn't mean that the batch is actually available, as old batches might get pruned.
    #[tracing::instrument(skip_all, fields(l1_batch = %number))]
    pub async fn wait_until_persisted(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::OrCanceled<BatchStoreState> {
        let t = metrics::BATCH_STORE.wait_until_persisted.start();

        let state = sync::wait_for(ctx, &mut self.persistent.persisted(), |persisted| {
            number < persisted.next()
        })
        .await?
        .clone();

        t.observe();
        Ok(state)
    }

    fn scrape_metrics(&self) -> metrics::BatchStoreState {
        let m = metrics::BatchStoreState::default();
        let inner = self.inner.borrow();
        m.next_queued_batch.set(inner.queued.next().0);
        m.next_persisted_batch.set(inner.persisted.next().0);
        m
    }
}
