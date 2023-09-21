//! Simple signal reporting primitive. A building block for `Scope` and `Ctx`.
//! Can also be used outside of the crate, but only together with `Ctx`.
use crate::ctx;

/// Communication channel over which a signal can be sent only once.
/// Useful for reporting very simple events.
pub struct Once(tokio::sync::Semaphore);

impl Default for Once {
    fn default() -> Once {
        Once::new()
    }
}

impl Once {
    /// Constructs a new `Once` channel.
    pub fn new() -> Self {
        Self(tokio::sync::Semaphore::new(0))
    }

    /// Sends the signal, waking all tasks awaiting for recv().
    ///
    /// After this call recv().await will always return immediately.
    /// After this call any subsequent call to send() is a noop.
    pub fn send(&self) {
        self.0.close();
    }

    /// Waits for the first call to send().
    /// Cancel-safe.
    pub(super) async fn cancel_safe_recv(&self) {
        // We await for the underlying semaphore to get closed.
        // This is the only possible outcome, because we never add
        // any permits to the semaphore.
        let res = self.0.acquire().await;
        debug_assert!(res.is_err());
    }

    /// Waits for the signal to be sent.
    pub async fn recv(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        ctx.wait(self.cancel_safe_recv()).await
    }

    /// Checks if send() was already called.
    pub fn try_recv(&self) -> bool {
        self.0.is_closed()
    }
}
