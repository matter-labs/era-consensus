//! Storage metrics.
use std::time;

#[derive(Debug, vise::Metrics)]
#[metrics(prefix = "zksync_consensus_storage_persistent_block_store")]
pub(super) struct PersistentBlockStore {
    /// Latency of a successful `state()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) state_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `block()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) block_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `store_next_block()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) store_next_block_latency: vise::Histogram<time::Duration>,
}

#[vise::register]
pub(super) static PERSISTENT_BLOCK_STORE: vise::Global<PersistentBlockStore> = vise::Global::new();

#[derive(Debug, vise::Metrics)]
#[metrics(prefix = "zksync_consensus_storage_block_store")]
pub(super) struct BlockStore {
    /// BlockNumber of the last queued block.
    pub(super) last_queued_block: vise::Gauge<u64>,
    /// BlockNumber of the last persisted block.
    pub(super) last_persisted_block: vise::Gauge<u64>,
}
