/// Metrics related to the gossiping of L1 batch votes.
#[derive(Debug, vise::Metrics)]
#[metrics(prefix = "network_gossip_batch_votes")]
pub(crate) struct BatchVotesMetrics {
    /// Number of members in the attester committee.
    pub(crate) committee_size: vise::Gauge<usize>,

    /// Number of votes added to the tally.
    ///
    /// Its rate of change should correlate with the attester committee size,
    /// save for any new joiner casting their historic votes in a burst.
    pub(crate) votes_added: vise::Counter,

    /// The minimum batch number we still expect votes for.
    ///
    /// This should go up as the main node indicates the finalisation of batches,
    /// or as soon as batch QCs are found and persisted.
    pub(crate) min_batch_number: vise::Gauge<u64>,

    /// Batch number in the last vote added to the register.
    ///
    /// This should go up as L1 batches are created, save for any temporary
    /// outlier from lagging attesters or ones sending votes far in the future.
    pub(crate) last_added_vote_batch_number: vise::Gauge<u64>,

    /// Batch number of the last batch signed by this attester.
    pub(crate) last_signed_batch_number: vise::Gauge<u64>,
}

#[vise::register]
pub(super) static BATCH_VOTES_METRICS: vise::Global<BatchVotesMetrics> = vise::Global::new();
