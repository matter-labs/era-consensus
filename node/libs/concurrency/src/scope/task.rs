//! `Task` is an internal representation of a routine spawned within a scope.
//! Each task is an async or sync function which owns a reference to `TerminateGuard`
//! (to keep the scope alive) and might return an error. When the task's routine returns
//! an error, the scope gets immediately canceled via `State::set_err` call.
//!
//! The task's routine is required to complete ASAP (i.e. without blocking or awaiting)
//! once the scope is canceled (see `Ctx` for details).
//!
//! We have 2 types of scope tasks:
//! * main task - the scope is considered to have finished its work once all main tasks return.
//!   Main tasks can spawn more main tasks if they need more concurrency (i.e. main tasks form a
//!   tree). Once main tasks are finished the scope is canceled automatically, even if all main
//!   tasks succeeded.
//! * background tasks - tasks which are "running in the background": performing periodical
//!   operations, handling requests from the main tasks, etc. You can consider them as
//!   "microservices" used by the main tasks to do their work. Background tasks can spawn more
//!   background tasks if needed.
//!
//! NOTE: although background tasks are not considered to be doing the "work of the scope" they
//! are still required to complete gracefully once the scope gets canceled: if all main tasks
//! complete succesfully, and one of the background tasks returns an error afterwards,
//! `scope::run!` will still return an error.
//!
//! NOTE: although background tasks shouldn't spawn main tasks, the current API of `Scope`
//! doesn't prevent that. In the future we might want to improve this API, but for now
//! spawning a main task via `Scope::spawn`/`Scope::spawn_blocking` from a background thread
//! may fall back to spawning a background task instead, in case all the main tasks have already
//! been completed.
//!
//! Task can be either async or blocking:
//! * Async tasks are Futures executed via `Task::run`. They MUSN'T call blocking operations,
//!   because they are executed on a shared thread pool.
//! * Blocking tasks are `FnOnce()` functions/closures exeucted via `Task::run_blocking`. Blocking
//!   task MUST be executed on a dedicated thread rather than a shared thread pool.
//! * All functions which perform blocking calls should be documented as blocking.
//!   If a function has multiple versions and the async version is called `<f>`, then the sync
//!   non-blocking version should be called `try_<f>` and sync blocking version should be called
//!   `<f>_blocking`. Except for `Scope::spawn_blocking` which is sync and non-blocking, and just
//!   spawns a new blocking task.
//! * Async calls can be turned into blocking calls via `ctx::block_on` function.
//!   It has `pub(crate)` visibility, therefore if you want to add a new blocking primitive, it
//!   should be placed in the `concurrency` crate.
use crate::scope;
use std::{future::Future, sync::Arc};

/// Error returned by `Task::run`/`Task::run_blocking` instead of
/// the actual error returned by the task's routine.
/// The actual error will be result of the `scope::run!` call.
#[derive(Debug, thiserror::Error)]
#[error("task terminated with an error")]
pub(crate) struct Terminated;

/// Internal representation of a routine spawned in the scope.
/// It owns a different guard of the scope's state, depending
/// on the task type(main or background).
pub(super) enum Task<E: 'static> {
    /// Main task.
    Main(Arc<scope::CancelGuard<E>>),
    /// Background task.
    Background(Arc<scope::TerminateGuard<E>>),
}

impl<E: 'static + Send> Task<E> {
    /// Getter of the guard owned by the task.
    fn guard(&self) -> &scope::TerminateGuard<E> {
        match self {
            Self::Background(g) => g.as_ref(),
            Self::Main(g) => g.terminate_guard().as_ref(),
        }
    }

    /// Runs an async task in the scope. Should be executed on a shared threadpool.
    /// If the task's routine succeeds, `run` completes immediately and returns the result.
    /// If the task's routine returns an error:
    /// 1. The error is passed to the scope's state (via `State::set_err`).
    /// 2. Scope's termination is awaited (we drop the task's guard before that to avoid a
    ///    deadlock).
    /// 3. Scope returns Terminated.
    pub(super) async fn run<T>(
        self,
        f: impl Future<Output = Result<T, E>>,
    ) -> Result<T, Terminated> {
        match f.await {
            Ok(v) => Ok(v),
            Err(err) => {
                self.guard().set_err(err);
                Err(Terminated)
            }
        }
    }

    /// Runs an sync blocking task in the scope. MUST be executed on a dedicated thread.
    /// See `Task::run` for behavior. See module docs for desciption of blocking tasks.
    pub(super) fn run_blocking<T>(self, f: impl FnOnce() -> Result<T, E>) -> Result<T, Terminated> {
        match f() {
            Ok(v) => Ok(v),
            Err(err) => {
                self.guard().set_err(err);
                Err(Terminated)
            }
        }
    }
}
