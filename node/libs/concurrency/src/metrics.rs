//! Prometheus metrics utilities.

use std::time::Duration;

use vise::Gauge;

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
