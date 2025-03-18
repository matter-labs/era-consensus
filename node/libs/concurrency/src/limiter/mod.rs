//! Rate limiter which supports delayed permit consumption.
use std::{fmt, sync::Mutex};

use crate::{ctx, sync, time};

#[cfg(test)]
mod tests;

/// Rate at which limiter should refresh the permits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    /// Maximal number of permits in a limiter.
    /// Whenever limiter already has `burst` fresh permits,
    /// refreshing is paused.
    pub burst: usize,
    /// Period at which permits are refreshed.
    /// Every `refresh` time, 1 token is added to the limiter
    /// (unless there is already `burst` fresh tokens).
    pub refresh: time::Duration,
}

impl Rate {
    /// Infinite refresh rate.
    pub const INF: Rate = Rate {
        burst: usize::MAX,
        refresh: time::Duration::ZERO,
    };
}

/// Representation of Duration in nanoseconds.
/// Duration is equivalent to {seconds:i64,nanos:i32},
/// so Nanos is a strictly bigger type.
type Nanos = i128;

/// Saturating conversion from positive `Nanos` to `Duration`.
fn duration_or_max(d: Nanos) -> time::Duration {
    debug_assert!(d >= 0);
    const NANOS_MAX: Nanos = time::Duration::MAX.whole_nanoseconds();
    const NANOS_PER_SEC: Nanos = 1000000000;
    if d > NANOS_MAX {
        return time::Duration::MAX;
    }
    time::Duration::new((d / NANOS_PER_SEC) as i64, (d % NANOS_PER_SEC) as i32)
}

/// Saturating conversion from positive i128 to usize.
fn usize_or_max(v: i128) -> usize {
    debug_assert!(v >= 0);
    v.try_into().unwrap_or(usize::MAX)
}

/// Mutable state of the Limiter.
/// Invariant: `0 <= reserved <= permits <= burst`.
struct State {
    /// Number of refresh periods which passed since limiter.start.
    /// We use it instead of just time::Instant, because it is easier
    /// to do arithmetics with it.
    /// It is i128 because pessimistically 1 refresh = 1 nanosecond,
    /// so refresh count has to be as expressive as Nanos.
    refresh_ticks: i128,
    /// Number of fresh permits.
    permits: usize,
    /// Number of reserved permits.
    reserved: usize,
}

impl State {
    /// Advances the state of the limiter to time `refresh_ticks`.
    fn advance(&mut self, refresh_ticks: i128, l: &Limiter) {
        // we use both `ctx.now()` and `ctx.sleep_until_deadline()` to determine
        // `refresh_ticks`, hence it is not obvious that refresh_ticks will be
        // always increasing at each call.
        if refresh_ticks < self.refresh_ticks {
            return;
        }
        let add = usize_or_max(refresh_ticks - self.refresh_ticks);
        // saturate_add is needed since a lot of time could have passed.
        self.permits = std::cmp::min(self.permits.saturating_add(add), l.burst);
        self.refresh_ticks = refresh_ticks;
    }
}

/// Rate limiter supporting delayed permit consumption.
/// Intuitively it works just like `tokio::Semaphore`
/// (with `rate.burst`==`capacity`) but there is a delay between
/// the moment the permit gets dropped and the moment it is again available
/// for acquiring: the dropped permits are delivered to a `refresher`
/// background task which returns them to the limiter one by one
/// every `rate.refresh` time.
///
/// For small `rate.refresh` period (i.e. high refresh frequency) it would
/// be inefficient to make the refresher task return the permits one by one,
/// so instead we simulate the refresher task lazily within the limiter state:
/// permit refreshing happens once per `acquire/Permit::drop` call and it simply
/// computes in O(1) time how many permits would have been refreshed by now by the refresher
/// task if we had one.
pub struct Limiter {
    /// Time at which the Limiter has been constructed.
    /// Number of refresh ticks is computed as `(now()-start)/refresh`.
    start: time::Instant,
    /// rate.refresh converted to Nanos.
    refresh: Nanos,
    /// Maximal number of permits in the limiter.
    /// Equivalent to `Semaphore::capacity()`.
    burst: usize,
    /// Mutable state of the limiter.
    state: Mutex<sync::watch::Sender<State>>,
    /// Mutex acquired by every `acquire()` call.
    /// Concurrent acquire() calls are executed synchronously
    /// using the fair queue provided by `sync::Mutex`.
    acquire: sync::Mutex<sync::watch::Receiver<State>>,
}

impl fmt::Debug for Limiter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Limiter").finish_non_exhaustive()
    }
}

/// Permit reservation returned by `Limit::acquire()`.
/// Represents a set of reserved permits.
/// You need to drop it for the permits to get consumed
/// (and only after consumption they can start refreshing).
pub struct Permit<'a> {
    /// Number of permits.
    permits: usize,
    /// Limiter which provided this permit.
    limiter: &'a Limiter,
    // Permit contains a reference to ctx, so that it can call `ctx.now()` in `drop()`.
    ctx: &'a ctx::Ctx,
}

impl Drop for Permit<'_> {
    /// Consumes the reserved permits.
    fn drop(&mut self) {
        if self.permits == 0 {
            return;
        }
        self.limiter.state.lock().unwrap().send_modify(|s| {
            // Division is ok, since l.refresh is positive.
            let refresh_ticks =
                (self.ctx.now() - self.limiter.start).whole_nanoseconds() / self.limiter.refresh;
            s.advance(refresh_ticks, self.limiter);
            // No overflow, since both have been incremented when permit was constructed.
            s.reserved -= self.permits;
            s.permits -= self.permits;
        });
    }
}

impl Limiter {
    /// Constructs a new rate limiter.
    pub fn new(ctx: &ctx::Ctx, rate: Rate) -> Self {
        let (send, recv) = sync::watch::channel(State {
            permits: rate.burst,
            refresh_ticks: 0,
            reserved: 0,
        });
        Self {
            start: ctx.now(),
            refresh: rate.refresh.whole_nanoseconds(),
            burst: rate.burst,
            state: Mutex::new(send),
            acquire: sync::Mutex::new(recv),
        }
    }

    /// Acquires reservation for `permits` permits from the rate limiter.
    /// It blocks until enough permits are available.
    /// It is fair in a sense that in case a later acquire() call is
    /// executed, but for a smaller number of permits, it has to wait
    /// until the previous call (for a larger number of permits) completes.
    /// If acquire() call is cancelled, no permits are consumed or reserved.
    pub async fn acquire<'a>(
        &'a self,
        ctx: &'a ctx::Ctx,
        permits: usize,
    ) -> ctx::OrCanceled<Permit<'a>> {
        // Too many permits requested.
        if self.burst < permits {
            ctx.canceled().await;
            return Err(ctx::Canceled);
        }
        // Infinite refresh rate. This is a totally valid case, when someone wants to disable
        // rate limiting entirely.
        if self.refresh <= 0 {
            return Ok(Permit {
                permits: 0,
                ctx,
                limiter: self,
            });
        }
        // Acquire calls are executed sequentially.
        let mut acquire = sync::lock(ctx, &self.acquire).await?.into_async();
        let need = {
            // The reserved permits will start refreshing only after they are consumed.
            // So we have to wait until enough reservations are consumed, so that we know
            // precisely how long we have to wait for our permits to get refreshed.
            let state =
                sync::wait_for(ctx, &mut acquire, |s| self.burst - s.reserved >= permits).await?;
            // Now there is enough refreshing permits, compute how long we will need to wait for them.
            state.refresh_ticks + (state.reserved + permits).saturating_sub(state.permits) as i128
        };
        // Sleep until the permits get refreshed.
        if need > 0 {
            let deadline = match self
                .start
                .checked_add(duration_or_max(self.refresh.saturating_mul(need)))
            {
                Some(t) => time::Deadline::Finite(t),
                None => time::Deadline::Infinite,
            };
            ctx.sleep_until_deadline(deadline).await?;
        }
        // Reserve the refreshed permits.
        // It is important that we update the state only after the sleep,
        // because otherwise the permits could have been leaked.
        self.state.lock().unwrap().send_if_modified(|s| {
            s.advance(need, self);
            s.reserved += permits;
            // Nobody waits anyway.
            false
        });
        Ok(Permit {
            permits,
            ctx,
            limiter: self,
        })
    }
}
