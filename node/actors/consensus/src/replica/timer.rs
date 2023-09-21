use super::StateMachine;
use concurrency::{ctx, time};
use once_cell::sync::Lazy;
use tracing::instrument;

static VIEW_TIMEOUT: Lazy<prometheus::Gauge> = Lazy::new(|| {
    prometheus::register_gauge!(
        "consensus_replica__view_timeout",
        "currently set timeout after which replica will proceed to the next view",
    )
    .unwrap()
});

impl StateMachine {
    /// The base duration of the timeout.
    pub(crate) const BASE_DURATION: time::Duration = time::Duration::milliseconds(1000);

    /// Resets the timer. On every timeout we double the duration, starting from a given base duration.
    /// This is a simple exponential backoff.
    #[instrument(level = "trace", ret)]
    pub(crate) fn reset_timer(&mut self, ctx: &ctx::Ctx) {
        let timeout =
            Self::BASE_DURATION * 2u32.pow((self.view.0 - self.high_qc.message.view.0) as u32);
        VIEW_TIMEOUT.set(timeout.as_seconds_f64());
        self.timeout_deadline = time::Deadline::Finite(ctx.now() + timeout);
    }
}
