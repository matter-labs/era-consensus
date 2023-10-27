//! Implementation of Structured Concurrency (both sync and async) with cancellation,
//! based on Golang errgroup (https://pkg.go.dev/golang.org/x/sync/errgroup).
//! Instead of "errgroup" we use name "scope" to be consistent with `std::thread::scope`.
//!
//! Scope represents a concurrent computation bounded by lifetime `'env`.
//! It should be constructed only via `run!` and `run_blocking!` macros.
//! Scope consists of a tree of tasks, which are either blocking or async.
//! You pass the root task as an argument to `run!`/`run_blocking!` and
//! each task can spawn more tasks within this scope.
//!
//! `run!`/`run_blocking!` returns only after all the tasks in the scope are
//! completed. If any of the tasks returns an error, `run!`/`run_blocking!` returns
//! the error of the task which failed first (all subsequent errors are silently
//! dropped). If all the tasks succeed, the result of the root task is returned.
//!
//! When the first task returns an error the scope's context is canceled, and all the
//! other tasks are notified (which means that they should complete ASAP). The
//! scope can be also canceled externally if the context of the caller gets canceled.
//! Scope can also get canceled once all the main tasks complete (see `Task`).
//!
//! Scope tasks have access to all objects with lifetime bigger than `'env`,
//! in particular, they have access to the local variables of the caller.
//! Scope tasks do not have access to each others' local variables because
//! there is no guarantee on the order in which the scope's tasks will be terminated.
//!
//! Scopes can be nested - you just need to call `run!`/`run_blocking` from within
//! the task. `Scope` is an implementation of a concept called Structured Concurrency,
//! which allows and promotes making concurrency an implementation detail of your functions.
//! Therefore it is highly discouraged to pass `&Scope` as an argument to functions - if
//! some function needs concurrency internally, it should rather run its own scope internally.
// TODO(gprusak): add more tests
// TODO(gprusak): add simple useful real-life examples
// TODO(gprusak): add a style guide on how to use scopes.
#![allow(unsafe_code)]
use crate::{ctx, time};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Weak},
};

mod macros;
mod must_complete;
mod state;
mod task;

pub use macros::*;
use state::{CancelGuard, OrPanic, State, TerminateGuard};
use task::{Task, Terminated};
use tracing::Instrument as _;

#[cfg(test)]
mod tests;

/// Equivalent of `std::thread::ScopedJoinHandle`.
/// Allows for awaiting for task completion and retrieving the result.
pub struct JoinHandle<'env, T>(
    tokio::task::JoinHandle<Result<T, Terminated>>,
    std::marker::PhantomData<fn(&'env ()) -> &'env ()>,
);

/// Spawnable blocking closure.
type BoxBlocking<'env, T> = Box<dyn 'env + Send + FnOnce() -> T>;

/// Spawns an blocking task on a dedicated thread of the tokio runtime.
/// It is unsafe because it is responsibility of the caller to make sure
/// that the task completes within `'env` lifetime.
unsafe fn spawn_blocking<'env, T: Send + 'static>(
    f: BoxBlocking<'env, Result<T, Terminated>>,
) -> JoinHandle<'env, T> {
    let f = std::mem::transmute::<BoxBlocking<'env, _>, BoxBlocking<'static, _>>(f);
    let span = tracing::Span::current();
    JoinHandle(
        tokio::task::spawn_blocking(move || {
            let _guard = span.enter();
            f()
        }),
        std::marker::PhantomData,
    )
}

/// Spawnable async closure.
type BoxFuture<'env, T> = Pin<Box<dyn 'env + Send + Future<Output = T>>>;

/// Spawns an async task in a shared thread pool of the tokio runtime.
/// It is unsafe because it is responsibility of the caller to make sure
/// that the task completes within `'env` lifetime.
unsafe fn spawn<'env, T: Send + 'static>(
    f: BoxFuture<'env, Result<T, Terminated>>,
) -> JoinHandle<'env, T> {
    let f = std::mem::transmute::<BoxFuture<'env, _>, BoxFuture<'static, _>>(f);
    JoinHandle(
        tokio::task::spawn(f.in_current_span()),
        std::marker::PhantomData,
    )
}

impl<'env, T> JoinHandle<'env, T> {
    /// Awaits completion of the task.
    /// Returns the result of the task.
    /// Returns `Canceled` if the context has been canceled before the task.
    /// Panics if the awaited task panicked.
    ///
    /// Caller is expected to provide their local context as an argument, which
    /// is neccessarily a descendant of the scope's context (because JoinHandle cannot
    /// leave the scope's lifetime). There might arise a race condition between
    /// the cancelation of `ctx` and termination of the task, in which case
    /// we await cancelation of `ctx` explicitly, so that the race condition is not
    /// observable to the caller.
    ///
    /// NOTE that the context of the spawned task may be different
    /// than the context of the awaiting task. You should pass your local
    /// context as an argument to `join`.
    pub fn join<'a>(
        self,
        ctx: &'a ctx::Ctx,
    ) -> ctx::CtxAware<impl 'a + Future<Output = ctx::OrCanceled<T>>>
    where
        T: 'a,
    {
        ctx::CtxAware(async move {
            if let Ok(v) = ctx.wait(self.0).await?.unwrap() {
                return Ok(v);
            }
            ctx.canceled().await;
            Err(ctx::Canceled)
        })
    }

    /// Unconditional join used in run/run_blocking to await the root task.
    async fn join_raw(self) -> ctx::OrCanceled<T> {
        match self.0.await {
            Ok(Ok(v)) => Ok(v),
            _ => Err(ctx::Canceled),
        }
    }
}

/// Scope represents a concurrent computation bounded by lifetime `'env`.
///
/// Scope is constructed outside of the `run()` call, so it cannot contain a strong reference
/// to the guards because we await scope termination within `run()` call (which is triggered
/// by dropping the TerminateGuard).
///
/// Scope cannot be constructed inside of `run()` call because the root task takes
/// `&'env Scope` as an argument.
///
/// However the implementation guarantees that cancel_guard is non-null as long as there is at least 1
/// main task running, and that terminate_guard is non-null as long as ANY task is running.
pub struct Scope<'env, E: 'static> {
    /// Context of the scope. It references `State::ctx`.
    ctx: ctx::Ctx,
    /// Cancel guard, for spawning main tasks.
    cancel_guard: Weak<CancelGuard<E>>,
    /// Terminate guard, for spawning background tasks.
    terminate_guard: Weak<TerminateGuard<E>>,
    /// Makes Scope<'env,E> invariant in 'env.
    _env: std::marker::PhantomData<fn(&'env ()) -> &'env ()>,
}

impl<'env, E: 'static + Send> Scope<'env, E> {
    /// Constructs a new main task within the scope.
    /// Note that it may return a background task though,
    /// in case it is called from within a background task
    /// after all the main tasks complete.
    /// TODO(gprusak): current scope API is not strict enough to prevent that,
    /// but it can be implemented if needed.
    fn main_task(&self) -> Task<E> {
        match self.cancel_guard.upgrade() {
            Some(guard) => Task::Main(guard),
            None => self.bg_task(),
        }
    }

    /// Constructs a new background task within the scope.
    fn bg_task(&self) -> Task<E> {
        // This unwrap is safe if `spawn` is called from within any scope task.
        // See `Scope` for more details about why it is done this way.
        Task::Background(self.terminate_guard.upgrade().unwrap())
    }

    /// Spawns a blocking main task in the scope.
    /// This is NOT a blocking function, so it can be called from async context.
    pub fn spawn_blocking<T: 'static + Send>(
        &self,
        f: impl 'env + Send + FnOnce() -> Result<T, E>,
    ) -> JoinHandle<'env, T> {
        let task = self.main_task();
        unsafe { spawn_blocking(Box::new(move || task.run_blocking(f))) }
    }

    /// Spawns a blocking background task in the scope.
    /// This is NOT a blocking function, so it can be called from async context.
    pub fn spawn_bg_blocking<T: 'static + Send>(
        &self,
        f: impl 'env + Send + FnOnce() -> Result<T, E>,
    ) -> JoinHandle<'env, T> {
        let task = self.bg_task();
        unsafe { spawn_blocking(Box::new(move || task.run_blocking(f))) }
    }

    /// Spawns an async main task in the scope.
    pub fn spawn<T: 'static + Send>(
        &self,
        f: impl 'env + Send + Future<Output = Result<T, E>>,
    ) -> JoinHandle<'env, T> {
        unsafe { spawn(Box::pin(self.main_task().run(f))) }
    }

    /// Spawns an async background task in the scope.
    pub fn spawn_bg<T: 'static + Send>(
        &self,
        f: impl 'env + Send + Future<Output = Result<T, E>>,
    ) -> JoinHandle<'env, T> {
        unsafe { spawn(Box::pin(self.bg_task().run(f))) }
    }

    /// Immediately cancels the scope's context.
    /// Normally all main tasks are expected to complete successfully
    /// before the context is canceled.
    /// You can use this method if you want to cancel immediately without
    /// returning an error (so that the whole scope can actually terminate
    /// succesfully).
    pub fn cancel(&self) {
        self.ctx.cancel();
    }

    /// not public; used by run! and run_blocking! macros.
    /// Constructs a new scope.
    /// It is used only in expansion of `run!`/`run_blocking!` and
    /// constructs a temporary object, which effectively binds 'env
    /// to be the lifetime of the `run`/`run_blocking` call.
    #[doc(hidden)]
    pub fn new(parent: &ctx::Ctx) -> Self {
        Self {
            ctx: parent.clone().child(time::Deadline::Infinite),
            cancel_guard: Weak::new(),
            terminate_guard: Weak::new(),
            _env: std::marker::PhantomData,
        }
    }

    /// not public; used by run! macro.
    ///
    /// Constructs a future running a new async scope.
    /// Future returns once all the scope tasks have completed.
    /// This future is expected to be immediately awaited at the call site.
    ///
    /// `self` argument is expected to be literally `Scope::new(...)`,
    /// so that it is roughly equivalent to just constructing a new Scope
    /// object within `run_blocking()` function, while binding `'env` to
    /// the lifetime of the call.
    ///
    /// `root_task` is executed as a root task of this scope.
    ///
    // Safety:
    // - we are assuming that `run` is only called via `run!` macro
    // - <'env> is exactly equal to the lifetime of the `Scope::new(...).run(...).await` call.
    // - in particular `run(...)` cannot be assigned to a local variable, because
    //   it would reference the temporal `Scope::new(...)` object.
    // - the returned future uses `must_complete::Guard` so it will abort
    //   the whole process if dropped before completion.
    // - before the first `poll()` call of the `run(...)` future may be forgotten (via `std::mem::forget`)
    //   directly or indirectly and that's safe, because no unsafe code has been executed yet.
    // - when `poll()` is called the first time, the future has to be already pinned, which
    //   implies that it is guaranteed to be dropped eventually and that it won't be moved to
    //   another memory location until then. All the references contained by the future also stay
    //   valid until then.
    // - the root task is embedded into the `run(...)` future and all the tasks spawned
    //   transitively use references stored in the root task, so they stay valid as well,
    //   until `run()` future is dropped.
    #[doc(hidden)]
    pub async fn run<T: 'static + Send, F, Fut>(&'env mut self, root_task: F) -> Result<T, E>
    where
        F: 'env + FnOnce(&'env ctx::Ctx, &'env Self) -> Fut,
        Fut: 'env + Send + Future<Output = Result<T, E>>,
    {
        // Abort if run() future is dropped before completion.
        let must_complete = must_complete::Guard;

        let guard = Arc::new(State::make(self.ctx.clone()));
        self.cancel_guard = Arc::downgrade(&guard);
        self.terminate_guard = Arc::downgrade(guard.terminate_guard());
        let state = guard.terminate_guard().state().clone();
        // Spawn the root task. We cannot run it directly in this task,
        // because if thr root task panicked, we wouldn't be able to
        // wait for other tasks to finish.
        let root_task = self.spawn(root_task(&self.ctx, self));
        // Once we spawned the root task we can drop the guard.
        drop(guard);
        // Await for the completion of the root_task.
        let root_task_result = root_task.join_raw().await;
        // Wait for the scope termination.
        state.terminated().await;

        // All tasks completed.
        // WARNING: NO `await` IS ALLOWED BELOW THIS LINE.
        must_complete.defuse();

        // Return the result of the root_task, the error, or propagate the panic.
        match state.take_err() {
            // All tasks have completed successfully, so in particular root_task has returned Ok.
            None => Ok(root_task_result.unwrap()),
            // One of the tasks returned an error, but no panic occurred.
            Some(OrPanic::Err(err)) => Err(err),
            // Note that panic is propagated only once all of the tasks are run to completion.
            Some(OrPanic::Panic) => {
                panic!("one of the tasks panicked, look for a stack trace above")
            }
        }
    }

    /// not public; used by run_blocking! macro.
    ///
    /// Runs a blocking scope, with a blocking `root_task`.
    /// This function is blocking, so should be called only from a blocking
    /// task (in particular, not from async code).
    /// Behaves analogically to `run`.
    #[doc(hidden)]
    pub fn run_blocking<T: 'static + Send, F: Send>(&'env mut self, root_task: F) -> Result<T, E>
    where
        E: 'static + Send,
        F: 'env + FnOnce(&'env ctx::Ctx, &'env Self) -> Result<T, E>,
    {
        let guard = Arc::new(State::make(self.ctx.clone()));
        self.cancel_guard = Arc::downgrade(&guard);
        self.terminate_guard = Arc::downgrade(guard.terminate_guard());
        let state = guard.terminate_guard().state().clone();
        // Spawn the root task. We cannot run it directly in this task,
        // because if thr root task panicked, we wouldn't be able to
        // wait for other tasks to finish.
        let root_task = self.spawn_blocking(|| root_task(&self.ctx, self));
        // Once we spawned the root task we can drop the guard.
        drop(guard);
        // Await for the completion of the root_task.
        let root_task_result = ctx::block_on(root_task.join_raw());
        // Wait for the scope termination.
        ctx::block_on(state.terminated());

        // Return the result of the root_task, the error, or propagate the panic.
        match state.take_err() {
            // All tasks have completed successfully, so in particular root_task has returned Ok.
            None => Ok(root_task_result.unwrap()),
            // One of the tasks returned an error, but no panic occurred.
            Some(OrPanic::Err(err)) => Err(err),
            // Note that panic is propagated only once all of the tasks are run to completion.
            Some(OrPanic::Panic) => {
                panic!("one of the tasks panicked, look for a stack trace above")
            }
        }
    }
}

/// Spawns the blocking closure `f` and unconditionally awaits for completion.
/// Panics if `f` panics.
/// Aborts if dropped before completion.
pub async fn wait_blocking<'a, T: 'static + Send>(f: impl 'a + Send + FnOnce() -> T) -> T {
    let must_complete = must_complete::Guard;
    let res = unsafe { spawn_blocking(Box::new(|| Ok(f()))) }
        .join_raw()
        .await;
    must_complete.defuse();
    res.expect("awaited task panicked")
}
