//! Prometheus metrics utilities.

use std::time::Duration;
use vise::Gauge;

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
