//! Time module provides a non-global clock, which is owned by ctx::Ctx.
//! Functions which use system clock directly are non-hermetic, which
//! makes them effectively non-deterministic and hard to test.
//!
//! Clock provides 2 types of time reads:
//! 1. now() (aka POSIX CLOCK_MONOTONIC, aka std::time::Instant)
//!    time as perceived by the machine making the measurement.
//!    The subsequent calls to now() are guaranteed to return monotonic
//!    results. It should be used for measuring the latency of operations
//!    as observed by the machine. The time::Instant itself doesn't
//!    translate to any specific timestamp, so it is not meaningful for
//!    anyone other than the machine doing the measurement.
//! 2. now_utc() (aka POSIX CLOCK_REALTIME, aka std::time::SystemTime)
//!    expected to approximate the (global) UTC time.
//!    There is NO guarantee that the subsequent reads will be monotonic,
//!    as CLOCK_REALTIME is configurable in the OS settings, or can be updated
//!    during NTP sync. Should be used whenever you need to communicate a timestamp
//!    over the network, or store it for later use. Remember that clocks
//!    of different machines are not perfectly synchronized, and in extreme
//!    cases can be totally skewed.
#![allow(clippy::float_arithmetic)]
use crate::time;
use once_cell::sync::Lazy;
use std::{
    fmt,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

// Instant doesn't have a deterministic constructor.
// However since Instant is not convertible to an unix timestamp,
// we can snapshot Instant::now() once and treat it as a constant.
// All observable effects will be then deterministic.
static FAKE_CLOCK_MONO_START: Lazy<time::Instant> = Lazy::new(time::Instant::now);

// An arbitrary non-trivial deterministic UTC timestamp.
// It is deterministic, so that the tests using fake clock have reproducible results.
// It was chosen at random, to make it harder for users to depend on a specific value.
const FAKE_CLOCK_UTC_START: time::Utc = time::Utc(time::Duration::new(891082933, 154890243));

/// Realtime clock.
#[derive(Clone)]
pub struct RealClock;

impl RealClock {
    /// Current time according to the monotone clock.
    pub fn now(&self) -> time::Instant {
        // We use `now()` from tokio, so that `tokio::time::pause()`
        // works in tests.
        tokio::time::Instant::now().into_std().into()
    }
    /// Current time according to the system/walltime clock.
    pub fn now_utc(&self) -> time::Utc {
        use std::time::SystemTime as T;
        time::Utc(match T::now().duration_since(T::UNIX_EPOCH) {
            Ok(duration) => time::Duration::try_from(duration).unwrap(),
            Err(err) => -time::Duration::try_from(err.duration()).unwrap(),
        })
    }
}

struct ManualState {
    /// `mono` keeps the current time of the monotonic clock.
    /// It is wrapped in watch::Sender, so that the value can
    /// be observed from the clock::sleep() futures.
    mono: watch::Sender<time::Instant>,
    /// Current UTC time.
    utc: time::Utc,
    /// We need to keep it so that mono.send() always succeeds.
    _mono_recv: watch::Receiver<time::Instant>,
    /// Whether time should be auto advanced at sleep() calls.
    /// Effectively makes sleep() calls non-blocking.
    advance_on_sleep: bool,
}

impl fmt::Debug for ManualState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManualState")
            .field("utc", &self.utc)
            .field("advance_on_sleep", &self.advance_on_sleep)
            .finish()
    }
}

/// Fake clock which supports manually advancing the time.
#[derive(Debug, Clone)]
pub struct ManualClock(Arc<Mutex<ManualState>>);

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ManualClock {
    /// Constructs a manual clock set to a default value of now.
    pub fn new() -> Self {
        let (mono, _mono_recv) = watch::channel(*FAKE_CLOCK_MONO_START);
        Self(Arc::new(Mutex::new(ManualState {
            utc: FAKE_CLOCK_UTC_START,
            mono,
            _mono_recv,
            advance_on_sleep: false,
        })))
    }

    /// Current time according to the monotone clock.
    pub fn now(&self) -> time::Instant {
        *self.0.lock().unwrap().mono.borrow()
    }

    /// Current time according to the system/walltime clock.
    pub fn now_utc(&self) -> time::Utc {
        self.0.lock().unwrap().utc
    }

    /// Advances monotonic and utc clocks by `d`.
    pub fn advance(&self, d: time::Duration) {
        let mut this = self.0.lock().unwrap();
        assert!(d >= time::Duration::ZERO);
        if d == time::Duration::ZERO {
            return;
        }
        let now = *this.mono.borrow();
        this.mono.send(now + d).unwrap();
        this.utc += d;
    }

    /// Advances monotonic and utc clocks to `t`.
    /// Noop if `t` is already in the past.
    pub fn advance_until(&self, t: time::Instant) {
        let mut this = self.0.lock().unwrap();
        let now = *this.mono.borrow();
        if t <= now {
            return;
        }
        this.mono.send(t).unwrap();
        this.utc += t - now;
    }

    /// Enables auto advancing time on sleep.
    /// Affects only sleep calls started AFTER this call.
    pub fn set_advance_on_sleep(&self) {
        self.0.lock().unwrap().advance_on_sleep = true;
    }

    /// Sets the UTC clock to a specific value.
    /// It doesn't affect the monotone clock.
    pub fn set_utc(&self, utc: time::Utc) {
        self.0.lock().unwrap().utc = utc;
    }
}

/// Clock which is running `speedup` times faster than the realtime.
/// (if `speedup` is <1, then it is actually slower).
#[derive(Clone)]
pub struct AffineClock {
    speedup: f64,
}

impl AffineClock {
    /// Constructs an AffineClock.
    pub fn new(speedup: f64) -> Self {
        Self { speedup }
    }

    /// Current time according to the monotone clock.
    pub fn now(&self) -> time::Instant {
        self.real_to_fake(time::Instant::now())
    }

    /// Current time according to the system/walltime clock.
    pub fn now_utc(&self) -> time::Utc {
        self.real_to_fake_utc(time::Instant::now())
    }

    fn real_to_fake(&self, t: time::Instant) -> time::Instant {
        *FAKE_CLOCK_MONO_START + (t - *FAKE_CLOCK_MONO_START) * self.speedup
    }
    fn real_to_fake_utc(&self, t: time::Instant) -> time::Utc {
        FAKE_CLOCK_UTC_START + (t - *FAKE_CLOCK_MONO_START) * self.speedup
    }
    fn fake_to_real(&self, t: time::Instant) -> time::Instant {
        *FAKE_CLOCK_MONO_START + (t - *FAKE_CLOCK_MONO_START) / self.speedup
    }
}

/// An abstract clock.
/// We use a concrete enum rather than a trait to
/// avoid abstract method call in runtime.
#[derive(Clone)]
pub enum Clock {
    /// Realtime clock.
    Real(RealClock),
    /// Affine clock.
    Affine(AffineClock),
    /// Manual clock.
    Manual(ManualClock),
}

impl From<RealClock> for Clock {
    fn from(c: RealClock) -> Self {
        Self::Real(c)
    }
}

impl From<AffineClock> for Clock {
    fn from(c: AffineClock) -> Self {
        Self::Affine(c)
    }
}

impl From<ManualClock> for Clock {
    fn from(c: ManualClock) -> Self {
        Self::Manual(c)
    }
}

impl Clock {
    /// Current time according to the monotone clock.
    pub fn now(&self) -> time::Instant {
        match self {
            Self::Real(c) => c.now(),
            Self::Affine(c) => c.now(),
            Self::Manual(c) => c.now(),
        }
    }

    /// Current time according to the system/walltime clock.
    pub fn now_utc(&self) -> time::Utc {
        match self {
            Self::Real(c) => c.now_utc(),
            Self::Affine(c) => c.now_utc(),
            Self::Manual(c) => c.now_utc(),
        }
    }

    /// Blocks until `d` passes.
    /// Cancel-safe.
    pub(crate) async fn sleep(&self, d: time::Duration) {
        match self {
            Self::Real(_) => tokio::time::sleep(d.try_into().unwrap()).await,
            Self::Affine(affine) => {
                tokio::time::sleep((d / affine.speedup).try_into().unwrap()).await
            }
            Self::Manual(manual) => {
                if manual.0.lock().unwrap().advance_on_sleep {
                    manual.advance(d);
                    return;
                }
                let mut watch = manual.0.lock().unwrap().mono.subscribe();
                let t = *watch.borrow() + d;
                while *watch.borrow() < t {
                    watch.changed().await.unwrap();
                }
            }
        }
    }

    /// Blocks until deadline `t`.
    /// Cancel-safe.
    pub(crate) async fn sleep_until(&self, t: time::Deadline, may_autoadvance: bool) {
        let time::Deadline::Finite(t) = t else {
            std::future::pending().await
        };
        match self {
            Self::Real(_) => tokio::time::sleep_until(t.into_inner().into()).await,
            Self::Affine(affine) => {
                tokio::time::sleep_until(affine.fake_to_real(t).into_inner().into()).await
            }
            Self::Manual(manual) => {
                if may_autoadvance && manual.0.lock().unwrap().advance_on_sleep {
                    manual.advance_until(t);
                    return;
                }
                let mut watch = manual.0.lock().unwrap().mono.subscribe();
                while *watch.borrow() < t {
                    watch.changed().await.unwrap();
                }
            }
        }
    }
}
