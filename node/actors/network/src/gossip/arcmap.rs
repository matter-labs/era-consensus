//! Multimap of pointers indexed by `node::PublicKey`.
//! Used to maintain a collection GetBlock rpc clients.
//! TODO(gprusak): consider upgrading PoolWatch instead.
use std::sync::Mutex;
use std::collections::HashMap;
use zksync_consensus_roles::node;
use std::sync::Arc;

/// ArcMap
pub(crate) struct ArcMap<T>(Mutex<HashMap<node::PublicKey, Vec<Arc<T>>>>);

impl<T> Default for ArcMap<T> {
    fn default() -> Self {
        Self(Mutex::default())
    }
}

impl<T> ArcMap<T> {
    /// Fetches any pointer for the given key.
    pub(crate) fn get_any(&self, key: &node::PublicKey) -> Option<Arc<T>> {
        self.0.lock().unwrap().get(key)?.first().cloned()
    }

    /// Insert a pointer.
    pub(crate) fn insert(&self, key: node::PublicKey, p: Arc<T>) {
        self.0.lock().unwrap().entry(key).or_default().push(p);
    }

    /// Removes a pointer.
    pub(crate) fn remove(&self, key: node::PublicKey, p: Arc<T>) {
        let mut this = self.0.lock().unwrap();
        use std::collections::hash_map::Entry;
        let Entry::Occupied(mut e) = this.entry(key) else {
            return;
        };
        e.get_mut().retain(|c| !Arc::ptr_eq(&p, c));
        if e.get_mut().is_empty() {
            e.remove();
        }
    }
}
