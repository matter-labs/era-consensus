use super::StateMachine;
use crate::metrics;
use tracing::instrument;
use zksync_concurrency::{ctx, metrics::LatencyGaugeExt as _, time};
use zksync_consensus_roles::validator;

impl StateMachine {
    /// The base duration of the timeout.
    pub(crate) const BASE_DURATION: time::Duration = time::Duration::milliseconds(2000);

    /// Resets the timer. On every timeout we double the duration, starting from a given base duration.
    /// This is a simple exponential backoff.
    #[instrument(level = "trace", ret)]
    pub(crate) fn reset_timer(&mut self, ctx: &ctx::Ctx) {
        let final_view = match self.high_qc.as_ref() {
            Some(qc) => qc.view().number.next(),
            None => validator::ViewNumber(0),
        };
        let timeout = Self::BASE_DURATION * 2u32.pow((self.view.0 - final_view.0) as u32);

        metrics::METRICS.replica_view_timeout.set_latency(timeout);
        self.timeout_deadline = time::Deadline::Finite(ctx.now() + timeout);
    }
}
