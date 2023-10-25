use crate::{io::SyncState, pool::PoolWatch, rpc, watch::Watch};
use concurrency::sync::{self, watch, Mutex};
use roles::{node, validator};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

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
    pub dynamic_inbound_limit: u64,
    /// Inbound connections that should be unconditionally accepted.
    pub static_inbound: HashSet<node::PublicKey>,
    /// Outbound connections that the node should actively try to
    /// establish and maintain.
    pub static_outbound: HashMap<node::PublicKey, std::net::SocketAddr>,
    /// Enables pings sent over the gossip network.
    pub enable_pings: bool,
}

type ClientMap<R> = Mutex<HashMap<node::PublicKey, Arc<rpc::Client<R>>>>;

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
    /// Subscriber for `SyncState` updates.
    pub(crate) sync_state: Option<watch::Receiver<SyncState>>,
    /// Clients for `get_block` requests for each currently active peer.
    pub(crate) get_block_clients: ClientMap<rpc::sync_blocks::GetBlockRpc>,
}

impl State {
    /// Constructs a new State.
    pub(crate) fn new(cfg: Config, sync_state: Option<watch::Receiver<SyncState>>) -> Self {
        Self {
            inbound: PoolWatch::new(
                cfg.static_inbound.clone(),
                cfg.dynamic_inbound_limit as usize,
            ),
            outbound: PoolWatch::new(cfg.static_outbound.keys().cloned().collect(), 0),
            validator_addrs: ValidatorAddrsWatch::default(),
            sync_state,
            get_block_clients: ClientMap::default(),
            cfg,
        }
    }
}
