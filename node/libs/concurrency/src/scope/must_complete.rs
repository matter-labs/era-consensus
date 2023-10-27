//! must_complete wraps a future, so that it aborts if it is dropped before completion.
//! Note that it ABORTS the process rather than just panic, so that we get a strong guarantee
//! of completion in both `panic=abort` and `panic=unwind` compilation modes.
//!
//! Possibility that a future can be dropped/aborted at every await makes the control flow unnecessarily complicated.
//! In fact, only few basic futures (like io primitives) actually need to be abortable, so
//! that they can be put together into a tokio::select block. All the higher level logic
//! would greatly benefit (in terms of readability and bug-resistance) from being non-abortable.
//! Rust doesn't support linear types as of now, so best we can do is a runtime check.

/// Guard which aborts the process when dropped.
/// Use `Guard::defuse()` to avoid aborting. 
pub(super) struct Guard;

impl Guard {
    pub fn defuse(self) {
        std::mem::forget(self)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        // We always abort here, no matter if compiled with panic=abort or panic=unwind.
        eprintln!("dropped a non-abortable future before completion");
        eprintln!("backtrace:\n{}", std::backtrace::Backtrace::force_capture());
        std::process::abort();
    }
}
