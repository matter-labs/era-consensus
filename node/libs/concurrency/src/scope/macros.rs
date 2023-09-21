//! A reimplementation of std::thread::scope which supports scope cancellation.
//! Cancellation is implemented via implicit context passing (`ctx::Ctx`).
//! Each scope creates a new child context of the current thread.
//! The context is cancelled as soon as ANY of the tasks spawned within the scope returns an error.
//! The whole scope returns the first returned error, or the result of the root task iff all the
//! tasks exited successfully.
//!
//! We use a macro syntax to fix 'env lifetime. `std::thread::scope` is using higher-order traits (`for<'a>`)
//! and compiler lifetime deduction, which has been extended enough to cover this use case.
//! Unfortunately this deduction is too weak to implement our Scope on top of `std::thread::scope`.
//! We use a more crude (but way more usable) mechanism, by explicitly constructing a temporary local object,
//! which binds the 'env lifetime exactly to the `scope::run!` expression. This approach supports
//! both sync and async scope implementation. See `Scope` for more details.

/// Runs an async scope.
#[macro_export]
macro_rules! run {
    ($ctx:expr, $f:expr) => {{
        $crate::scope::Scope::new($ctx).run($f)
    }};
}

/// Runs a blocking scope.
#[macro_export]
macro_rules! run_blocking {
    ($ctx:expr, $f:expr) => {{
        $crate::scope::Scope::new($ctx).run_blocking($f)
    }};
}

pub use run;
pub use run_blocking;
