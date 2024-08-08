//! Storage metrics.
use std::time;

#[derive(Debug, vise::Metrics)]
#[metrics(prefix = "zksync_consensus_storage_persistent_batch_store")]
pub(super) struct PersistentBatchStore {
    /// Latency of a successful `get_batch()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) batch_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `next_batch_to_attest_latency()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) next_batch_to_attest_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `get_batch_to_sign()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) batch_to_sign_latency: vise::Histogram<time::Duration>,
}

#[vise::register]
pub(super) static PERSISTENT_BATCH_STORE: vise::Global<PersistentBatchStore> = vise::Global::new();

#[derive(Debug, vise::Metrics)]
#[metrics(prefix = "zksync_consensus_storage_batch_store")]
pub(super) struct BatchStore {
    /// Overall latency of a `queue_batch()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) queue_batch: vise::Histogram<time::Duration>,
    /// Overall latency of a `persist_batch_qc()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) persist_batch_qc: vise::Histogram<time::Duration>,
    /// Overall latency of a `wait_until_queued()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) wait_until_queued: vise::Histogram<time::Duration>,
    /// Overall latency of a `wait_until_persisted()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) wait_until_persisted: vise::Histogram<time::Duration>,
    /// Last persisted batch QC.
    pub(super) last_persisted_batch_qc: vise::Gauge<u64>,
}

#[vise::register]
pub(super) static BATCH_STORE: vise::Global<BatchStore> = vise::Global::new();

#[derive(Debug, vise::Metrics)]
#[metrics(prefix = "zksync_consensus_storage_batch_store")]
pub(super) struct BatchStoreState {
    /// BatchNumber of the next batch to queue.
    pub(super) next_queued_batch: vise::Gauge<u64>,
    /// BatchNumber of the next batch to persist.
    pub(super) next_persisted_batch: vise::Gauge<u64>,
}
