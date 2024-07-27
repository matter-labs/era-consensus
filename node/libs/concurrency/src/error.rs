//! Generalization of anyhow::Context to more structured errors.
use std::fmt::Display;

/// Trait complementary to `anyhow::Context` which allows for
/// adding context to error types which contain `anyhow::Error`.
///
/// If an error type implements both `Wrap` and `From<anyhow::Error>`
/// you should be careful to NOT use `context()` instead of `wrap()`,
/// because `context()` will just hide all the error details.
pub trait Wrap: Sized {
    /// Appends context `c` to the error.
    fn wrap<C: Display + Send + Sync + 'static>(self, c: C) -> Self {
        self.with_wrap(|| c)
    }
    /// Appends context `f()` to the error.
    fn with_wrap<C: Display + Send + Sync + 'static, F: FnOnce() -> C>(self, f: F) -> Self;
}

impl<T, E: Wrap> Wrap for Result<T, E> {
    fn with_wrap<C: Display + Send + Sync + 'static, F: FnOnce() -> C>(self, f: F) -> Self {
        self.map_err(|err| err.with_wrap(f))
    }
}
