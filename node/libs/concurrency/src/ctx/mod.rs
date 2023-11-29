//! Implementation of the golang Context (https://pkg.go.dev/context).
//! You can think of context as a runtime equivalent of the rust lifetime:
//! As soon as its context is canceled the function should return ASAP,
//! without doing any further blocking calls.
//!
//! It is NOT possible to extend the context provided by the caller, who defines
//! how long it allows the function call to execute.
//!
//! Context is essentially a cancellation token which should be passed
//! down the call stack, so that it can be awaited together with every blocking call:
//! Instead of "awaiting for new data on the channel", you "await for new data on the channel OR
//! for context to get canceled". This way you can implement graceful shutdown
//! in a very uniform way.
//!
//! Contrary to the golang implementation, we pass the context implicitly
//! in the thread-local memory. Implicit passing may look like magic, however
//! * it is built on top of `tokio::Runtime` which is also passed implicitly,
//!   so the concept should be familiar for the tokio users.
//! * it prevents misuse of context, as what we actually try to model here
//!   is a reader monad, which in essence is equivalent to implicit argument passing
//!   (https://hackage.haskell.org/package/mtl-2.3.1/docs/Control-Monad-Reader.html)
//! * it presumably makes it easier to onboard new users, without having to add an explicit
//!   context argument to all functions in their codebase.
use crate::{signal, time};
use std::{fmt, future::Future, pin::Pin, sync::Arc, task};

pub mod channel;
mod clock;
mod rng;
mod testonly;
#[cfg(test)]
mod tests;

pub use clock::*;
pub use testonly::*;

/// Contexts are composed into a tree via `_parent` link.
/// We maintain an invariant `_parent.deadline <= deadline`.
/// If a parent gets canceled, the child also gets canceled immediately afterwards,
/// although not atomically. If deadline passes the context also gets canceled.
///
/// The cascade cancellation is implemented by spawning a tokio
/// task awaiting for parent to be canceled and canceling the child afterwards
/// (it also awaits for deadline and the child itself to be canceled just in case, to avoid memory leaks).
pub struct Ctx(Arc<Inner>);

/// Inner representation of the context.
struct Inner {
    clock: Clock,
    rng_provider: rng::Provider,
    /// Signal sent once this context is canceled.
    canceled: Arc<signal::Once>,
    /// Deadline after which the context will be automatically canceled.
    deadline: time::Deadline,
    /// Parent context.
    _parent: Option<Arc<Inner>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Automatically cancel the context when State is dropped.
        // This wakes the task awaiting parent cancellation, so that it doesn't leak.
        // Note that since children keep a reference to the parent, no
        // cascade cancellation will happen here.
        self.canceled.send();
    }
}

/// Error returned when the blocking operation was interrupted
/// due to context getting canceled.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("canceled")]
pub struct Canceled;

/// Wraps result with ErrCancel as an error.
pub type OrCanceled<T> = Result<T, Canceled>;

/// Blocks the current thread until future f is completed, using
/// the local tokio runtime. Use this function to generate a blocking
/// version of async context-aware primitives.
/// Blocking.
#[track_caller]
pub(crate) fn block_on<F: Future>(f: F) -> F::Output {
    tokio::runtime::Handle::current().block_on(f)
}

/// Constructs a top-level context.
/// Should be called only at the start of the `main()` function of the binary.
pub fn root() -> Ctx {
    Ctx(Arc::new(Inner {
        clock: RealClock.into(),
        rng_provider: rng::Provider::real(),
        canceled: Arc::new(signal::Once::new()),
        deadline: time::Deadline::Infinite,
        _parent: None,
    }))
}

impl fmt::Debug for Ctx {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Ctx").finish_non_exhaustive()
    }
}

/// Context-aware future, that an async task can await and
/// sync task can block on.
#[must_use = "futures do nothing unless you `.await` or poll them"]
#[pin_project::pin_project]
pub struct CtxAware<F>(#[pin] pub(crate) F);

impl<F: Future> Future for CtxAware<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        self.project().0.poll(cx)
    }
}

impl<F: Future> CtxAware<F> {
    /// Blocks the current thread until the future is completed.
    pub fn block(self) -> F::Output {
        block_on(self)
    }
}

impl Ctx {
    /// Clones the context.
    /// `Ctx` doesn't implement `Clone` so that it is not possible to
    /// clone it outside of this crate.
    pub(crate) fn clone(&self) -> Self {
        Self(self.0.clone())
    }

    /// Constructs a new child context.
    pub(crate) fn child(&self, deadline: time::Deadline) -> Self {
        self.child_with_clock(self.0.clock.clone(), deadline)
    }

    fn child_with_clock(&self, clock: Clock, deadline: time::Deadline) -> Self {
        let deadline = std::cmp::min(self.0.deadline, deadline);
        let parent_canceled = self.0.canceled.clone();
        let child_canceled = Arc::new(signal::Once::new());
        let child = Self(Arc::new(Inner {
            clock: clock.clone(),
            rng_provider: self.0.rng_provider.split(),
            canceled: child_canceled.clone(),
            deadline,
            _parent: Some(self.0.clone()),
        }));
        // Spawn a task propagating task cancelation.
        // This task takes references to only to the `canceled` signals
        // of parent and child to avoid a reference loop (rather than
        // the whole context object) to avoid a memory leak:
        // context is automatically canceled when dropped, which
        // guarantees that this task eventually completes.
        tokio::spawn(async move {
            tokio::select! {
                _ = clock.sleep_until(deadline,false) => child_canceled.send(),
                _ = parent_canceled.cancel_safe_recv() => child_canceled.send(),
                _ = child_canceled.cancel_safe_recv() => {},
            }
        });
        child
    }
    /// Cascade cancels this context and all the descendants.
    pub(crate) fn cancel(&self) {
        self.0.canceled.send();
    }

    /// Awaits until this context gets canceled.
    pub fn canceled(&self) -> CtxAware<impl '_ + Future<Output = ()>> {
        CtxAware(self.0.canceled.cancel_safe_recv())
    }

    /// Awaits until the local context gets canceled. Unlike [`Self::canceled()`], the returned
    /// future has a static lifetime.
    pub fn canceled_owned(&self) -> impl Future<Output = ()> {
        let canceled = self.0.canceled.clone();
        async move { canceled.cancel_safe_recv().await }
    }

    /// Checks if this context is still active (i.e., not canceled).
    pub fn is_active(&self) -> bool {
        !self.0.canceled.try_recv()
    }

    /// The time at which this context will be canceled.
    /// The task should use it to schedule its work accordingly.
    /// Remember that this is just a hint, because the local context
    /// may get canceled before the deadline.
    pub fn deadline(&self) -> time::Deadline {
        self.0.deadline
    }

    /// Awaits until the provided future `fut` completes, or the context gets canceled.
    /// `fut` is required to be cancel-safe. It logically doesn't make sense to call this method
    /// for context-aware futures, since they can handle context cancellation already.
    pub fn wait<'a, F: 'a + Future>(
        &'a self,
        fut: F,
    ) -> CtxAware<impl 'a + Future<Output = OrCanceled<F::Output>>> {
        CtxAware(async {
            tokio::select! {
                output = fut => Ok(output),
                () = self.0.canceled.cancel_safe_recv() => Err(Canceled),
            }
        })
    }

    /// Constructs a sub-context with deadline `d`.
    pub fn with_deadline(&self, d: time::Deadline) -> Self {
        self.child(d)
    }

    /// Constructs a sub-context with deadline `now() + d`.
    pub fn with_timeout(&self, d: time::Duration) -> Self {
        self.child((self.now() + d).into())
    }

    /// Current time according to the monotone clock.
    pub fn now(&self) -> time::Instant {
        self.0.clock.now()
    }

    /// Current time according to the system/walltime clock.
    pub fn now_utc(&self) -> time::Utc {
        self.0.clock.now_utc()
    }

    /// Waits for a specific time.
    pub fn sleep(&self, d: time::Duration) -> CtxAware<impl '_ + Future<Output = OrCanceled<()>>> {
        self.wait(self.0.clock.sleep(d))
    }

    /// Blocks until `t`.
    pub fn sleep_until(
        &self,
        t: time::Instant,
    ) -> CtxAware<impl '_ + Future<Output = OrCanceled<()>>> {
        self.wait(self.0.clock.sleep_until(t.into(), true))
    }

    /// Blocks until deadline `t`.
    pub fn sleep_until_deadline(
        &self,
        t: time::Deadline,
    ) -> CtxAware<impl '_ + Future<Output = OrCanceled<()>>> {
        self.wait(self.0.clock.sleep_until(t, true))
    }

    /// Constructs a cryptographically-safe random number generator,
    /// * in prod, seeded with system entropy
    ///   WARNING: do not store the result, but rather create new RNG
    ///     whenever you do computation requiring entropy source.
    ///     call to rng() is moderately expensive, so don't call it separately
    ///     for every single bit of entropy though.
    ///   TODO(gprusak): for now we do not have use cases for reseeding
    ///     but perhaps eventually we should returns an auto-reseeding RNG instead.
    /// * in test, seeded deterministically
    ///     TODO(gprusak): this is not a perfect determinism, because multiple
    ///     tasks currently are allowed to access the same ctx, so the order in
    ///     which they call rng() is not deterministic. To fix this we
    ///     would need to move the Provider to task-local storage.
    pub fn rng(&self) -> rand::rngs::StdRng {
        self.0.rng_provider.rng()
    }
}

/// anyhow::Error + "canceled" variant.
/// Useful for working with concurrent code which doesn't need structured errors,
/// but needs to handle cancelation explicitly.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Context has been canceled before call completion.
    #[error(transparent)]
    Canceled(#[from] Canceled),
    /// Other error.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl crate::error::Wrap for Error {
    fn with_wrap<C: std::fmt::Display + Send + Sync + 'static, F: FnOnce() -> C>(
        self,
        f: F,
    ) -> Self {
        match self {
            Error::Internal(err) => Error::Internal(err.context(f())),
            err => err,
        }
    }
}
