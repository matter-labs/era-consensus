use super::StateMachine;
use crate::metrics;
use tracing::instrument;
use zksync_concurrency::{ctx, metrics::LatencyGaugeExt as _, time};
use zksync_consensus_roles::validator;

impl StateMachine {
    /// The base duration of the timeout.
    pub(crate) const BASE_DURATION: time::Duration = time::Duration::milliseconds(2000);
    /// Max duration of the timeout.
    /// Consensus is unusable with this range of timeout anyway,
    /// however to make debugging easier we bound it to a specific value.
    pub(crate) const MAX_DURATION: time::Duration = time::Duration::seconds(1000000);

    /// Resets the timer. On every timeout we double the duration, starting from a given base duration.
    /// This is a simple exponential backoff.
    #[instrument(level = "trace", ret)]
    pub(crate) fn reset_timer(&mut self, ctx: &ctx::Ctx) {
        let final_view = match self.high_qc.as_ref() {
            Some(qc) => qc.view().number.next(),
            None => validator::ViewNumber(0),
        };
        let f = self
            .view
            .0
            .saturating_sub(final_view.0)
            .try_into()
            .unwrap_or(u32::MAX);
        let f = 2u64.saturating_pow(f).try_into().unwrap_or(i32::MAX);
        let timeout = Self::BASE_DURATION
            .saturating_mul(f)
            .min(Self::MAX_DURATION);

        metrics::METRICS.replica_view_timeout.set_latency(timeout);
        self.timeout_deadline = time::Deadline::Finite(ctx.now() + timeout);
    }
}
