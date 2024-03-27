//! An abstraction for a set of "connections" which constraints
//! which peers are allowed to connect.
use crate::watch::Watch;
use std::collections::HashSet;
use zksync_concurrency::sync;

/// Map with restrictions on the allowed keys.
/// This set consists of an arbitrary subset of `allowed` + up to `extra_limit` elements outside of
/// `allowed`.
#[derive(Clone)]
pub(crate) struct Pool<K, V> {
    extra_limit: usize,
    extra_count: usize,
    allowed: HashSet<K>,
    current: im::HashMap<K, V>,
}

impl<K, V> Pool<K, V> {
    /// Current pool state.
    pub(crate) fn current(&self) -> &im::HashMap<K, V> {
        &self.current
    }
}

/// Watch wrapper of the Pool.
/// Supports subscribing to the Pool membership changes.
pub(crate) struct PoolWatch<K, V>(Watch<Pool<K, V>>);

impl<K: std::hash::Hash + Eq + Clone, V: Clone> PoolWatch<K, V> {
    /// Constructs a new pool.
    pub(crate) fn new(allowed: HashSet<K>, extra_limit: usize) -> Self {
        Self(Watch::new(Pool {
            extra_limit,
            extra_count: 0,
            allowed,
            current: im::HashMap::new(),
        }))
    }

    /// Tries to insert an element to the set.
    /// Returns an error if
    /// * `v` is already in the set
    /// * `v` cannot be added due to size restrictions
    pub(crate) async fn insert(&self, k: K, v: V) -> anyhow::Result<()> {
        self.0
            .send_if_ok(|pool| {
                if pool.current.contains_key(&k) {
                    anyhow::bail!("already exists");
                }
                if !pool.allowed.contains(&k) {
                    if pool.extra_count >= pool.extra_limit {
                        anyhow::bail!("limit exceeded");
                    }
                    pool.extra_count += 1;
                }
                pool.current.insert(k, v);
                Ok(())
            })
            .await
    }

    /// Removes an element from the set.
    pub(crate) async fn remove(&self, k: &K) {
        self.0.lock().await.send_if_modified(|pool| {
            if pool.current.remove(k).is_none() {
                return false;
            }
            if !pool.allowed.contains(k) {
                pool.extra_count -= 1;
            }
            true
        });
    }

    /// Copy of the current pool state.
    pub(crate) fn current(&self) -> im::HashMap<K, V> {
        self.0.subscribe().borrow().current.clone()
    }

    /// Subscribes to the set changes.
    pub(crate) fn subscribe(&self) -> sync::watch::Receiver<Pool<K, V>> {
        self.0.subscribe()
    }
}
