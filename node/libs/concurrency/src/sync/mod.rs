//! Context-aware wrappers for tokio synchronization primitives.

use std::{fmt, marker::PhantomData, ops, sync::Arc};

pub use tokio::{
    sync::{
        watch, Mutex, MutexGuard, Notify, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore,
        SemaphorePermit,
    },
    task::yield_now,
};

use crate::{ctx, oneshot};

pub mod prunable_mpsc;
#[cfg(test)]
mod tests;

/// Error returned when a channel has been disconnected.
#[derive(Debug, thiserror::Error)]
#[error("disconnected")]
pub struct Disconnected;

/// Mutex guard that is `!Send` and thus cannot be held across an `await` point.
///
/// If you need to hold a guard across an `await` point, you can convert it to an "ordinary" guard
/// using [`Self::into_async()`].
pub struct LocalMutexGuard<'a, T> {
    inner: MutexGuard<'a, T>,
    _not_send: PhantomData<*mut ()>,
}

impl<'a, T> LocalMutexGuard<'a, T> {
    /// Unwraps this
    pub fn into_async(self) -> MutexGuard<'a, T> {
        self.inner
    }
}

impl<T> ops::Deref for LocalMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> ops::DerefMut for LocalMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// Locks a mutex, returning a guard which is NOT Send
/// (useful for ensuring that mutex is not held across await point).
/// Note that depending on a use case you might
/// want to wait unconditionally for a mutex to be locked
/// (when a mutex is guaranteed to be unlocked fast).
/// In such a case use `Mutex::lock()` instead.
pub async fn lock<'a, T>(
    ctx: &ctx::Ctx,
    mutex: &'a Mutex<T>,
) -> ctx::OrCanceled<LocalMutexGuard<'a, T>> {
    Ok(LocalMutexGuard {
        inner: ctx.wait(mutex.lock()).await?,
        _not_send: PhantomData,
    })
}

/// Locks a mutex, returning a 'static guard.
pub async fn lock_owned<T>(
    ctx: &ctx::Ctx,
    mutex: Arc<Mutex<T>>,
) -> ctx::OrCanceled<OwnedMutexGuard<T>> {
    ctx.wait(mutex.lock_owned()).await
}

/// Acquires a permit from a semaphore.
pub async fn acquire<'a>(
    ctx: &ctx::Ctx,
    semaphore: &'a Semaphore,
) -> ctx::OrCanceled<SemaphorePermit<'a>> {
    ctx.wait(async move {
        match semaphore.acquire().await {
            Ok(v) => v,
            Err(_) => std::future::pending().await,
        }
    })
    .await
}

/// Acquires `n` 'static permits from a semaphore.
pub async fn acquire_many_owned(
    ctx: &ctx::Ctx,
    semaphore: Arc<Semaphore>,
    n: u32,
) -> ctx::OrCanceled<OwnedSemaphorePermit> {
    ctx.wait(async move {
        match semaphore.acquire_many_owned(n).await {
            Ok(v) => v,
            Err(_) => std::future::pending().await,
        }
    })
    .await
}

/// Awaits a notification.
pub async fn notified(ctx: &ctx::Ctx, notify: &Notify) -> ctx::OrCanceled<()> {
    ctx.wait(notify.notified()).await
}

/// Sends the modified value iff `f()` returns `Ok`.
/// Forwards the result of `f()` to the caller.
pub fn try_send_modify<T, R, E>(
    send: &watch::Sender<T>,
    f: impl FnOnce(&mut T) -> Result<R, E>,
) -> Result<R, E> {
    let mut res = None;
    send.send_if_modified(|v| {
        let x = f(v);
        let s = x.is_ok();
        res = Some(x);
        s
    });
    // safe, since `res` is set by `send_if_modified`.
    res.unwrap()
}

/// Waits for a watch change notification.
/// Immediately borrows a reference to the new value.
pub async fn changed<'a, T>(
    ctx: &ctx::Ctx,
    recv: &'a mut watch::Receiver<T>,
) -> ctx::OrCanceled<watch::Ref<'a, T>> {
    ctx.wait(async {
        if recv.changed().await.is_err() {
            std::future::pending().await
        }
        recv.borrow_and_update()
    })
    .await
}

/// Waits until a watch contains a value satisfying
/// the given predicate. Returns a reference to this value.
pub async fn wait_for<'a, T>(
    ctx: &ctx::Ctx,
    recv: &'a mut watch::Receiver<T>,
    pred: impl Fn(&T) -> bool,
) -> ctx::OrCanceled<watch::Ref<'a, T>> {
    // TODO(gprusak): wait_for is not documented to be cancel-safe.
    // We should use changed() instead.
    if let Ok(res) = ctx.wait(recv.wait_for(pred)).await? {
        return Ok(res);
    }
    ctx.canceled().await;
    Err(ctx::Canceled)
}

/// Waits until predicate is different than `None`.
pub async fn wait_for_some<T, R>(
    ctx: &ctx::Ctx,
    recv: &mut watch::Receiver<T>,
    pred: impl Fn(&T) -> Option<R>,
) -> ctx::OrCanceled<R> {
    recv.mark_changed();
    loop {
        if let Some(v) = pred(&*changed(ctx, recv).await?) {
            return Ok(v);
        }
    }
}

struct ExclusiveLockInner<T> {
    value: T,
    drop_sender: oneshot::Sender<T>,
}

/// Exclusive lock on a value. The lock provides shared and exclusive references to the value,
/// and on drop returns the value to the corresponding [`ExclusiveLockReceiver`].
pub struct ExclusiveLock<T>(Option<ExclusiveLockInner<T>>);

impl<T: fmt::Debug> fmt::Debug for ExclusiveLock<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.0.as_ref().unwrap();
        // ^ `unwrap()` is safe; the value is only taken out on drop
        fmt::Debug::fmt(&inner.value, formatter)
    }
}

impl<T> Drop for ExclusiveLock<T> {
    fn drop(&mut self) {
        let inner = self.0.take().unwrap();
        // ^ `unwrap()`s are safe; we never take the value or sender before the drop
        let _ = inner.drop_sender.send(inner.value);
        // ^ We don't care if the corresponding guard is dropped.
    }
}

impl<T> ExclusiveLock<T> {
    /// Creates a new lock together with a receiver that will get the value once the lock is dropped.
    pub fn new(value: T) -> (Self, ExclusiveLockReceiver<T>) {
        let (drop_sender, drop_receiver) = oneshot::channel();
        let this = Self(Some(ExclusiveLockInner { value, drop_sender }));
        (this, ExclusiveLockReceiver(drop_receiver))
    }
}

impl<T> ops::Deref for ExclusiveLock<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0.as_ref().unwrap().value
        // ^ `unwrap()` is safe; the value is only taken out on drop
    }
}

impl<T> ops::DerefMut for ExclusiveLock<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0.as_mut().unwrap().value
        // ^ `unwrap()` is safe; the value is only taken out on drop
    }
}

/// Receiver for a value locked in [`ExclusiveLock`] that allows to receive the value back once
/// the lock is dropped.
pub struct ExclusiveLockReceiver<T>(oneshot::Receiver<T>);

impl<T> ExclusiveLockReceiver<T> {
    /// Waits until the corresponding [`ExclusiveLock`] is dropped, and returns the previously locked
    /// value afterwards.
    pub async fn wait(self, ctx: &ctx::Ctx) -> ctx::OrCanceled<T> {
        self.0.recv_or_disconnected(ctx).await.map(Result::unwrap)
        // ^ Since sender never disconnects without sending the value, the `unwrap()` is safe
    }
}
