//! Attestation metrics.
use super::Controller;
use std::sync::Weak;

/// Metrics related to the gossiping of L1 batch votes.
#[derive(Debug, vise::Metrics)]
#[metrics(prefix = "network_gossip_attestation")]
pub(crate) struct Metrics {
    /// Batch to be attested.
    pub(crate) batch_number: vise::Gauge<u64>,

    /// Number of members in the attester committee.
    pub(crate) committee_size: vise::Gauge<usize>,

    /// Number of votes collected for the current batch.
    pub(crate) votes_collected: vise::Gauge<usize>,

    /// Weight percentage (in range [0,1]) of votes collected for the current batch.
    pub(crate) weight_collected: vise::Gauge<f64>,
}

impl Metrics {
    /// Registers metrics to a global collector.
    pub(crate) fn register(ctrl: Weak<Controller>) {
        #[vise::register]
        static COLLECTOR: vise::Collector<Option<Metrics>> = vise::Collector::new();
        let res = COLLECTOR.before_scrape(move || {
            ctrl.upgrade().and_then(|ctrl| {
                let ctrl = (*ctrl.state.subscribe().borrow()).clone()?;
                let m = Metrics::default();
                m.batch_number.set(ctrl.info.batch_to_attest.number.0);
                m.committee_size.set(ctrl.info.committee.len());
                m.votes_collected.set(ctrl.votes.len());
                #[allow(clippy::float_arithmetic)]
                m.weight_collected
                    .set(ctrl.total_weight as f64 / ctrl.info.committee.total_weight() as f64);
                Some(m)
            })
        });
        if let Err(err) = res {
            tracing::warn!("Failed registering attestation metrics: {err:#}");
        }
    }
}
