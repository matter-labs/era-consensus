//! Metrics for the consensus module.

use std::time::Duration;
use vise::{Buckets, EncodeLabelSet, EncodeLabelValue, Family, Gauge, Histogram, Metrics, Unit};

const PAYLOAD_SIZE_BUCKETS: Buckets = Buckets::exponential(
    (4 * zksync_protobuf::kB) as f64..=(4 * zksync_protobuf::MB) as f64,
    4.0,
);

/// Label for a consensus message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
pub(crate) enum ConsensusMsgLabel {
    /// Label for a `LeaderPrepare` message.
    LeaderPrepare,
    /// Label for a `LeaderCommit` message.
    LeaderCommit,
    /// Label for a `ReplicaPrepare` message.
    ReplicaPrepare,
    /// Label for a `ReplicaCommit` message.
    ReplicaCommit,
}

impl ConsensusMsgLabel {
    /// Attaches a result to this label.
    pub(crate) fn with_result<E>(self, result: &Result<(), E>) -> ProcessingLatencyLabels {
        ProcessingLatencyLabels {
            r#type: self,
            result: match result {
                Ok(()) => ResultLabel::Ok,
                Err(_) => ResultLabel::Err,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
enum ResultLabel {
    Ok,
    Err,
}

/// Labels for processing latency metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet)]
pub(crate) struct ProcessingLatencyLabels {
    r#type: ConsensusMsgLabel,
    result: ResultLabel,
}

/// Metrics defined by the consensus module.
#[derive(Debug, Metrics)]
#[metrics(prefix = "consensus")]
pub(crate) struct ConsensusMetrics {
    /// Size of the proposed payload in bytes.
    #[metrics(buckets = PAYLOAD_SIZE_BUCKETS, unit = Unit::Bytes)]
    pub(crate) leader_proposal_payload_size: Histogram<usize>,
    /// Latency of the commit phase observed by the leader.
    #[metrics(buckets = Buckets::exponential(0.01..=20.0, 1.5), unit = Unit::Seconds)]
    pub(crate) leader_commit_phase_latency: Histogram<Duration>,
    /// Currently set timeout after which replica will proceed to the next view.
    #[metrics(unit = Unit::Seconds)]
    pub(crate) replica_view_timeout: Gauge<Duration>,
    /// Latency of processing messages by the replicas.
    #[metrics(buckets = Buckets::LATENCIES, unit = Unit::Seconds)]
    pub(crate) replica_processing_latency: Family<ProcessingLatencyLabels, Histogram<Duration>>,
    /// Latency of processing messages by the leader.
    #[metrics(buckets = Buckets::LATENCIES, unit = Unit::Seconds)]
    pub(crate) leader_processing_latency: Family<ProcessingLatencyLabels, Histogram<Duration>>,
    /// Number of the last finalized block observed by the node.
    pub(crate) finalized_block_number: Gauge<u64>,
    /// Number of the current view of the replica.
    #[metrics(unit = Unit::Seconds)]
    pub(crate) replica_view_number: Gauge<u64>,
}

/// Global instance of [`ConsensusMetrics`].
#[vise::register]
pub(crate) static METRICS: vise::Global<ConsensusMetrics> = vise::Global::new();
