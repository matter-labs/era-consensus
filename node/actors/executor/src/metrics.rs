//! Metrics defined by the executor module.

use vise::{Gauge, Metrics};

/// Metrics defined by the executor module.
#[derive(Debug, Metrics)]
#[metrics(prefix = "executor")]
pub(crate) struct ExecutorMetrics {
    /// Number of the last finalized block observed by the node.
    pub(crate) finalized_block_number: Gauge<u64>,
}

/// Global instance of [`ExecutorMetrics`].
#[vise::register]
pub(crate) static METRICS: vise::Global<ExecutorMetrics> = vise::Global::new();
