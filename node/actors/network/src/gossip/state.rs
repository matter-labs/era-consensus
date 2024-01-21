use crate::{pool::PoolWatch, rpc, watch::Watch};
use anyhow::Context as _;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::BlockStore;
use zksync_protobuf::kB;

/// Mapping from validator::PublicKey to a signed validator::NetAddress.
/// Represents the currents state of node's knowledge about the validator endpoints.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct ValidatorAddrs(
    pub(super) im::HashMap<validator::PublicKey, Arc<validator::Signed<validator::NetAddress>>>,
);

impl ValidatorAddrs {
    /// Gets a NetAddress for a given key.
    pub(crate) fn get(
        &self,
        key: &validator::PublicKey,
    ) -> Option<&Arc<validator::Signed<validator::NetAddress>>> {
        self.0.get(key)
    }

    /// Returns a set of entries of `self` which are newer than the entries in `b`.
    pub(super) fn get_newer(&self, b: &Self) -> Vec<Arc<validator::Signed<validator::NetAddress>>> {
        let mut newer = vec![];
        for (k, v) in &self.0 {
            if let Some(bv) = b.0.get(k) {
                if !v.msg.is_newer(&bv.msg) {
                    continue;
                }
            }
            newer.push(v.clone());
        }
        newer
    }

    /// Updates the discovery map with entries from `data`.
    /// It exits as soon as an invalid entry is found.
    /// `self` might get modified even if an error is returned
    /// (all entries verified so far are added).
    /// Returns true iff some new entry was added.
    pub(super) fn update(
        &mut self,
        validators: &validator::ValidatorSet,
        data: &[Arc<validator::Signed<validator::NetAddress>>],
    ) -> anyhow::Result<bool> {
        let mut changed = false;

        let mut done = HashSet::new();
        for d in data {
            // Disallow multiple entries for the same key:
            // it is important because a malicious validator may spam us with
            // new versions and verifying signatures is expensive.
            if done.contains(&d.key) {
                anyhow::bail!("duplicate entry for {:?}", d.key);
            }
            done.insert(d.key.clone());
            if !validators.contains(&d.key) {
                // We just skip the entries we are not interested in.
                // For now the set of validators is static, so we could treat this as an error,
                // however we eventually want the validator set to be dynamic.
                continue;
            }
            if let Some(x) = self.0.get(&d.key) {
                if !d.msg.is_newer(&x.msg) {
                    continue;
                }
            }
            d.verify()?;
            self.0.insert(d.key.clone(), d.clone());
            changed = true;
        }
        Ok(changed)
    }
}

/// Watch wrapper of ValidatorAddrs,
/// which supports subscribing to ValidatorAddr updates.
pub(crate) struct ValidatorAddrsWatch(Watch<ValidatorAddrs>);

impl Default for ValidatorAddrsWatch {
    fn default() -> Self {
        Self(Watch::new(ValidatorAddrs::default()))
    }
}

impl ValidatorAddrsWatch {
    /// Subscribes to ValidatorAddrs updates.
    pub(crate) fn subscribe(&self) -> sync::watch::Receiver<ValidatorAddrs> {
        self.0.subscribe()
    }

    /// Inserts data to ValidatorAddrs.
    /// Subscribers are notified iff at least 1 new entry has
    /// been inserted. Returns an error iff an invalid
    /// entry in `data` has been found. The provider of the
    /// invalid entry should be banned.
    pub(crate) async fn update(
        &self,
        validators: &validator::ValidatorSet,
        data: &[Arc<validator::Signed<validator::NetAddress>>],
    ) -> anyhow::Result<()> {
        let this = self.0.lock().await;
        let mut validator_addrs = this.borrow().clone();
        if validator_addrs.update(validators, data)? {
            this.send(validator_addrs).ok().unwrap();
        }
        Ok(())
    }
}

/// Gossip network configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Private key of the node, every node should have one.
    pub key: node::SecretKey,
    /// Limit on the number of inbound connections outside
    /// of the `static_inbound` set.
    pub dynamic_inbound_limit: usize,
    /// Inbound connections that should be unconditionally accepted.
    pub static_inbound: HashSet<node::PublicKey>,
    /// Outbound connections that the node should actively try to
    /// establish and maintain.
    pub static_outbound: HashMap<node::PublicKey, std::net::SocketAddr>,
}

/// Multimap of pointers indexed by `node::PublicKey`.
/// Used to maintain a collection GetBlock rpc clients.
/// TODO(gprusak): consider upgrading PoolWatch instead.
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

/// Gossip network state.
pub(crate) struct State {
    /// Gossip network configuration.
    pub(crate) cfg: Config,
    /// Currently open inbound connections.
    pub(crate) inbound: PoolWatch<node::PublicKey>,
    /// Currently open outbound connections.
    pub(crate) outbound: PoolWatch<node::PublicKey>,
    /// Current state of knowledge about validators' endpoints.
    pub(crate) validator_addrs: ValidatorAddrsWatch,
    /// Block store to serve `get_block` requests from.
    pub(crate) block_store: Arc<BlockStore>,
    /// Clients for `get_block` requests for each currently active peer.
    pub(crate) get_block_clients: ArcMap<rpc::Client<rpc::get_block::Rpc>>,
}

impl State {
    /// Constructs a new State.
    pub(crate) fn new(cfg: Config, block_store: Arc<BlockStore>) -> Self {
        Self {
            inbound: PoolWatch::new(cfg.static_inbound.clone(), cfg.dynamic_inbound_limit),
            outbound: PoolWatch::new(cfg.static_outbound.keys().cloned().collect(), 0),
            validator_addrs: ValidatorAddrsWatch::default(),
            block_store,
            get_block_clients: ArcMap::default(),
            cfg,
        }
    }

    pub(super) async fn get_block(
        &self,
        ctx: &ctx::Ctx,
        recipient: &node::PublicKey,
        number: validator::BlockNumber,
        max_block_size: usize,
    ) -> anyhow::Result<Option<validator::FinalBlock>> {
        Ok(self
            .get_block_clients
            .get_any(recipient)
            .context("recipient is unreachable")?
            .call(
                ctx,
                &rpc::get_block::Req(number),
                max_block_size.saturating_add(kB),
            )
            .await?
            .0)
    }
}
