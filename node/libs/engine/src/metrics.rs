use std::time;

#[vise::register]
pub(super) static ENGINE_INTERFACE: vise::Global<EngineInterface> = vise::Global::new();

#[derive(Debug, vise::Metrics)]
#[metrics(prefix = "zksync_consensus_engine_interface")]
pub(super) struct EngineInterface {
    /// Latency of a successful `get_block()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) get_block_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `queue_next_block()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) queue_next_block_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `verify_pregenesis_block()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) verify_pregenesis_block_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `verify_payload()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) verify_payload_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `propose_payload()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) propose_payload_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `get_state()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) get_state_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `set_state()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) set_state_latency: vise::Histogram<time::Duration>,
    /// Latency of a successful `push_tx()` call.
    #[metrics(unit = vise::Unit::Seconds, buckets = vise::Buckets::LATENCIES)]
    pub(super) push_tx_latency: vise::Histogram<time::Duration>,
}

#[vise::register]
pub(super) static BLOCK_STORE: vise::Collector<Option<BlockStore>> = vise::Collector::new();

#[derive(Debug, vise::Metrics)]
#[metrics(prefix = "zksync_consensus_engine_block_store")]
pub(super) struct BlockStore {
    /// BlockNumber of the next block to queue.
    pub(super) next_queued_block: vise::Gauge<u64>,
    /// BlockNumber of the next block to persist.
    pub(super) next_persisted_block: vise::Gauge<u64>,
    /// Number of blocks in the cache.
    pub(super) cache_size: vise::Gauge<u64>,
}
