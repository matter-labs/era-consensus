//! Testonly utilities for concurrent tests.
use std::{future::Future, io::IsTerminal as _};
use crate::ctx;
use crate::signal;
use std::sync::Arc;

/// Iff the current process is executed under
/// nextest in process-per-test mode, changes the behavior of the process to [panic=abort].
/// In particular it doesn't enable [panic=abort] when run via "cargo test".
/// Note that (unfortunately) some tests may expect a panic, so we cannot apply blindly
/// [panic=abort] in compilation time to all tests.
// TODO: investigate whether "-Zpanic-abort-tests" could replace this function once the flag
// becomes stable: https://github.com/rust-lang/rust/issues/67650, so we don't use it.
pub fn abort_on_panic() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .with_ansi(std::env::var("NO_COLOR").is_err() && std::io::stdout().is_terminal())
        .with_line_number(true)
        .try_init();

    // I don't know a way to set panic=abort for nextest builds in compilation time, so we set it
    // in runtime. https://nexte.st/book/env-vars.html#environment-variables-nextest-sets
    let Ok(nextest) = std::env::var("NEXTEST") else {
        return;
    };
    let Ok(nextest_execution_mode) = std::env::var("NEXTEST_EXECUTION_MODE") else {
        return;
    };
    if nextest != "1" || nextest_execution_mode != "process-per-test" {
        return;
    }
    tracing::info!("[panic=abort] enabled");
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        orig_hook(panic_info);
        std::process::abort();
    }));
}

/// Guard which has to be dropped before timeout is reached.
/// Otherwise the test will panic.
pub struct TimeoutGuard(Arc<signal::Once>);

impl Drop for TimeoutGuard {
    fn drop(&mut self) {
        self.0.send();
    }
}

/// Panics if (real time) timeout is reached before ctx is canceled.
pub fn set_timeout(timeout: time::Duration) -> TimeoutGuard {
    let done = Arc::new(signal::Once::new());
    let guard = TimeoutGuard(done.clone());
    tokio::task::spawn_blocking(move || {
        if let Err(ctx::Canceled) = done.recv(&ctx::root().with_timeout(timeout)).block() {
            panic!("TIMEOUT");
        }
    });
    guard
}

/// Executes a test under multiple configurations of the tokio runtime.
pub fn with_runtimes<Fut: Future>(test: impl Fn() -> Fut) {
    for (name, mut b) in [
        (
            "current_thread",
            tokio::runtime::Builder::new_current_thread(),
        ),
        ("multi_thread", tokio::runtime::Builder::new_multi_thread()),
    ] {
        tracing::info!("tokio runtime: {name}");
        let r = b.enable_all().build().unwrap();
        r.block_on(test());
    }
}
