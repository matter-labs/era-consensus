//! Wrapper of the tokio::sync::Watch for easier usage.
use zksync_concurrency::sync;

/// Wrapper of the tokio::sync::Watch.
pub(crate) struct Watch<T> {
    /// `sync::watch::Sender` contains synchronous mutex.
    /// We wrap it into an async mutex to wait for it asynchronously.
    send: sync::Mutex<sync::watch::Sender<T>>,
    /// Receiver outside of the mutex so that `subscribe()` can be nonblocking.
    recv: sync::watch::Receiver<T>,
}

impl<T> Watch<T> {
    /// Constructs a new watch with initial value `v`.
    pub(crate) fn new(v: T) -> Self {
        let (send, recv) = sync::watch::channel(v);
        Self {
            send: sync::Mutex::new(send),
            recv,
        }
    }

    /// Acquires a lock on the watch sender.
    pub(crate) async fn lock(&self) -> sync::MutexGuard<sync::watch::Sender<T>> {
        self.send.lock().await
    }

    /// Subscribes to the watch.
    pub(crate) fn subscribe(&self) -> sync::watch::Receiver<T> {
        self.recv.clone()
    }

    /// Applies `f` to the value and notifies
    /// subscribers iff `f()` returned `Ok`.
    pub(crate) async fn send_if_ok<R, E>(
        &self,
        f: impl FnOnce(&mut T) -> Result<R, E>,
    ) -> Result<R, E> {
        let mut mres = None;
        self.lock().await.send_if_modified(|v| {
            let res = f(v);
            let ok = res.is_ok();
            mres = Some(res);
            ok
        });
        mres.unwrap()
    }
}
