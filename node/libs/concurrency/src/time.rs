//! Well-defined alternatives to types in std::time.
//! Provides a signed Duration and UTC timestamps with
//! nanoseconds precision.

/// A signed Duration.
pub type Duration = time::Duration;

/// Monotonic clock time.
pub type Instant = time::Instant;

/// UTC time in nanoseconds precision.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Utc(pub(crate) Duration);

impl std::fmt::Debug for Utc {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        (std::time::SystemTime::UNIX_EPOCH + self.0).fmt(f)
    }
}

/// Start of the unix epoch.
pub const UNIX_EPOCH: Utc = Utc(Duration::ZERO);

/// Represents an optional deadline.
/// Isomorphic to `Option<time::Instant>`,
/// however the total ordering on `Deadline` is purposefully
/// defined, while on `Option<time::Instant>` it is accidental.
/// By the definition of derive(PartialEq), Finite(...) < Infinite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Deadline {
    /// Finite deadline.
    Finite(Instant),
    /// Infinite deadline.
    Infinite,
}

impl From<Instant> for Deadline {
    fn from(t: Instant) -> Self {
        Self::Finite(t)
    }
}

impl std::ops::Add<Duration> for Utc {
    type Output = Self;

    fn add(self, d: Duration) -> Self {
        Self(self.0 + d)
    }
}

impl std::ops::AddAssign<Duration> for Utc {
    fn add_assign(&mut self, d: Duration) {
        self.0 += d;
    }
}

impl std::ops::Sub<Duration> for Utc {
    type Output = Self;
    fn sub(self, d: Duration) -> Self {
        Self(self.0 - d)
    }
}

impl std::ops::SubAssign<Duration> for Utc {
    fn sub_assign(&mut self, d: Duration) {
        self.0 -= d;
    }
}

impl std::ops::Sub<Utc> for Utc {
    type Output = Duration;
    fn sub(self, b: Self) -> Duration {
        self.0 - b.0
    }
}
