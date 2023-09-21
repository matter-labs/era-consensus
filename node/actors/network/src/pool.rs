//! An abstraction for a set of "connections" which constraints
//! which peers are allowed to connect.
use crate::watch::Watch;
use concurrency::sync;
use std::collections::HashSet;

/// Set of elements of type T.
/// This set consists of an arbitrary subset of `allowed` + up to `extra_limit` elements outside of
/// `allowed`.
#[derive(Clone)]
pub(crate) struct Pool<T> {
    extra_limit: usize,
    extra_count: usize,
    allowed: HashSet<T>,
    current: HashSet<T>,
}

impl<T> Pool<T> {
    /// Returns a reference to the underlying set.
    #[allow(dead_code)]
    pub(crate) fn current(&self) -> &HashSet<T> {
        &self.current
    }
}

/// Watch wrapper of the Pool.
/// Supports subscribing to the Pool updates.
pub(crate) struct PoolWatch<T>(Watch<Pool<T>>);

impl<T: std::hash::Hash + PartialEq + Eq> PoolWatch<T> {
    /// Constructs a new pool.
    pub(crate) fn new(allowed: HashSet<T>, extra_limit: usize) -> Self {
        Self(Watch::new(Pool {
            extra_limit,
            extra_count: 0,
            allowed,
            current: HashSet::new(),
        }))
    }

    /// Tries to insert an element to the set.
    /// Returns an error if
    /// * `v` is already in the set
    /// * `v` cannot be added due to size restrictions
    pub(crate) async fn insert(&self, v: T) -> anyhow::Result<()> {
        self.0
            .send_if_ok(|pool| {
                if pool.current.contains(&v) {
                    anyhow::bail!("already exists");
                }
                if !pool.allowed.contains(&v) {
                    if pool.extra_count >= pool.extra_limit {
                        anyhow::bail!("limit exceeded");
                    }
                    pool.extra_count += 1;
                }
                pool.current.insert(v);
                Ok(())
            })
            .await
    }

    /// Removes an element from the set.
    pub(crate) async fn remove(&self, v: &T) {
        self.0.lock().await.send_if_modified(|pool| {
            if !pool.current.remove(v) {
                return false;
            }
            if !pool.allowed.contains(v) {
                pool.extra_count -= 1;
            }
            true
        });
    }

    /// Subscribes to the set changes.
    #[allow(dead_code)]
    pub(crate) fn subscribe(&self) -> sync::watch::Receiver<Pool<T>> {
        self.0.subscribe()
    }
}
