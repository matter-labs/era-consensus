//! `State` is an internal representation of the scope state.
//! It represents the lifecycle of the scope, but it is not bound to
//! the `Scope` object lifetime (in particular, it is not parametrized by `env` lifetime).
//!
//! Scope is canceled := scope's context is canceled.
//! Scope gets canceled when its `CancelGuard` gets dropped OR when any of the scope tasks
//! returns an error by calling `TerminateGuard::set_err`.
//! Scope gets terminated when its `TerminateGuard gets dropped.
//!
//! Note that if scope is terminated, then it is also canceled, because `CancelGuard` keeps
//! `TerminateGuard` alive.
//!
//! Note that scope can get canceled even if `CancelGuard` is still alive.
use crate::{ctx, signal};
use std::sync::{Arc, Mutex};

pub(super) enum OrPanic<E> {
    Err(E),
    Panic,
}

/// Internal representation of the scope.
pub(super) struct State<E> {
    /// Context of this scope.
    /// All tasks spawned in this scope are provided with this context.
    ctx: ctx::Ctx,
    /// First error returned by any task in the scope.
    err: Mutex<Option<OrPanic<E>>>,
    /// Signal sent once the scope is terminated.
    terminated: signal::Once,
}

impl<E> State<E> {
    /// Awaits termination of the scope.
    pub(super) async fn terminated(&self) {
        self.terminated.cancel_safe_recv().await
    }

    /// Takes out the error from the scope.
    /// Called after scope termination to return the error to
    /// the `scope::run!` caller.
    pub(super) fn take_err(&self) -> Option<OrPanic<E>> {
        debug_assert!(self.terminated.try_recv());
        std::mem::take(&mut *self.err.lock().unwrap())
    }

    /// Constructs a new `State` wrapped in `TerminateGuard` and `CancelGuard`.
    /// `State.ctx` is constructed as a child of `parent`.
    pub(super) fn make(ctx: ctx::Ctx) -> CancelGuard<E> {
        let state = State {
            ctx,
            err: Mutex::new(None),
            terminated: signal::Once::new(),
        };
        CancelGuard(Arc::new(TerminateGuard(Arc::new(state))))
    }
}

/// Wrapper of `State` which terminates the scope when dropped.
/// Each scope task must keep a reference to it to prevent
/// scope termination.
pub(super) struct TerminateGuard<E: 'static>(Arc<State<E>>);

impl<E: 'static> Drop for TerminateGuard<E> {
    fn drop(&mut self) {
        self.state().terminated.send();
    }
}

impl<E: 'static> TerminateGuard<E> {
    /// Getter of the guarded state.
    pub(super) fn state(&self) -> &Arc<State<E>> {
        &self.0
    }

    /// Sets the scope error if it is not already set.
    /// Panic overrides an error.
    /// Called by scope tasks which resulted with an error.
    /// It has a side effect of canceling the scope.
    pub(super) fn set_err(&self, err: OrPanic<E>) {
        let mut m = self.0.err.lock().unwrap();
        match (&*m, &err) {
            // Panic overrides an error, but error doesn't override an error.
            (Some(OrPanic::Panic), _) | (Some(OrPanic::Err(_)), OrPanic::Err(_)) => return,
            _ => {}
        }
        self.0.ctx.cancel();
        *m = Some(err);
    }
}

/// Wrapper of `State` which cancels the scope when dropped.
/// Each "main" scope task must keep a reference to it to prevent
/// scope premature cancelation. Note that the scope might get canceled
/// earlier anyway via `TerminateGuard::set_err`.
pub(super) struct CancelGuard<E: 'static>(Arc<TerminateGuard<E>>);

impl<E: 'static> Drop for CancelGuard<E> {
    fn drop(&mut self) {
        self.terminate_guard().state().ctx.cancel();
    }
}

impl<E: 'static> CancelGuard<E> {
    /// Getter of the terminate guard.
    pub(super) fn terminate_guard(&self) -> &Arc<TerminateGuard<E>> {
        &self.0
    }
}
