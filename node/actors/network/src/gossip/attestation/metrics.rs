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

#[vise::register]
pub(super) static METRICS: vise::Global<Metrics> = vise::Global::new();
