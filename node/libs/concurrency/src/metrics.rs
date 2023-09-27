//! Prometheus metrics utilities.

use std::time::Duration;
use vise::{Counter, EncodeLabelSet, EncodeLabelValue, Family, Gauge, Metrics, Unit};

/// Guard which increments the gauge when constructed
/// and decrements it when dropped.
pub struct GaugeGuard(Gauge);

impl From<Gauge> for GaugeGuard {
    fn from(gauge: Gauge) -> Self {
        gauge.inc_by(1);
        Self(gauge)
    }
}

impl Drop for GaugeGuard {
    fn drop(&mut self) {
        self.0.dec_by(1);
    }
}

/// Direction of a TCP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet, EncodeLabelValue)]
#[metrics(label = "direction", rename_all = "snake_case")]
pub(crate) enum Direction {
    /// Inbound connection.
    Inbound,
    /// Outbound connection.
    Outbound,
}

/// Metrics reported for TCP connections.
#[derive(Debug, Metrics)]
#[metrics(prefix = "concurrency_net_tcp")]
pub(crate) struct TcpMetrics {
    /// Total bytes sent over all TCP connections.
    #[metrics(unit = Unit::Bytes)]
    pub(crate) sent: Counter,
    /// Total bytes received over all TCP connections.
    #[metrics(unit = Unit::Bytes)]
    pub(crate) received: Counter,
    /// TCP connections established since the process started.
    pub(crate) established: Family<Direction, Counter>,
    /// Number of currently active TCP connections.
    pub(crate) active: Family<Direction, Gauge>,
}

/// TCP metrics instance.
#[vise::register]
pub(crate) static TCP_METRICS: vise::Global<TcpMetrics> = vise::Global::new();

/// Extension trait for latency histograms.
pub trait LatencyHistogramExt {
    /// Observes latency.
    fn observe_latency(&self, latency: time::Duration);
}

impl LatencyHistogramExt for vise::Histogram<Duration> {
    fn observe_latency(&self, latency: time::Duration) {
        let latency = Duration::try_from(latency).unwrap_or(Duration::ZERO);
        self.observe(latency);
    }
}

/// Extension trait for latency gauges.
pub trait LatencyGaugeExt {
    /// Sets the gauge value.
    fn set_latency(&self, latency: time::Duration);
}

impl LatencyGaugeExt for Gauge<Duration> {
    fn set_latency(&self, latency: time::Duration) {
        let latency = Duration::try_from(latency).unwrap_or(Duration::ZERO);
        self.set(latency);
    }
}
