//! Metrics for the consensus module.

use std::time::Duration;

use vise::{Buckets, EncodeLabelSet, EncodeLabelValue, Family, Gauge, Histogram, Metrics, Unit};

/// Label for a consensus message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue)]
#[metrics(rename_all = "snake_case")]
pub(crate) enum ConsensusMsgLabel {
    /// Label for a `LeaderProposal` message.
    LeaderProposal,
    /// Label for a `ReplicaCommit` message.
    ReplicaCommit,
    /// Label for a `ReplicaTimeout` message.
    ReplicaTimeout,
    /// Label for a `ReplicaNewView` message.
    ReplicaNewView,
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
    /// Number of the current view of the replica.
    pub(crate) replica_view_number: Gauge<u64>,
    /// Number of the last finalized block observed by the node.
    pub(crate) finalized_block_number: Gauge<u64>,
    /// Size of the proposed payload in bytes.
    #[metrics(buckets = Buckets::exponential(
        (4 * zksync_protobuf::kB) as f64..=(4 * zksync_protobuf::MB) as f64,
        2.0,
    ), unit = Unit::Bytes)]
    pub(crate) proposal_payload_size: Histogram<usize>,
    /// Latency of receiving a proposal as observed by the replica. Measures from
    /// the start of the view until we have a verified proposal.
    #[metrics(buckets = Buckets::exponential(0.125..=64.0, 2.0), unit = Unit::Seconds)]
    pub(crate) proposal_latency: Histogram<Duration>,
    /// Latency of committing to a block as observed by the replica. Measures from
    /// the start of the view until we send a commit vote.
    #[metrics(buckets = Buckets::exponential(0.125..=64.0, 2.0), unit = Unit::Seconds)]
    pub(crate) commit_latency: Histogram<Duration>,
    /// Latency of a single view as observed by the replica. Measures from
    /// the start of the view until the start of the next.
    #[metrics(buckets = Buckets::exponential(0.125..=64.0, 2.0), unit = Unit::Seconds)]
    pub(crate) view_latency: Histogram<Duration>,
    /// Latency of processing messages by the replicas.
    #[metrics(buckets = Buckets::LATENCIES, unit = Unit::Seconds)]
    pub(crate) message_processing_latency: Family<ProcessingLatencyLabels, Histogram<Duration>>,
}

/// Global instance of [`ConsensusMetrics`].
#[vise::register]
pub(crate) static METRICS: vise::Global<ConsensusMetrics> = vise::Global::new();
