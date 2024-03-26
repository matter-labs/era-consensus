//! Global state distributed by active validators, observed by all the nodes in the network.
use crate::watch::Watch;
use std::{collections::HashSet, sync::Arc};
use zksync_concurrency::{time,sync};
use zksync_consensus_roles::validator;

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

    pub(crate) async fn announce(
        &self,
        key: &validator::SecretKey,
        addr: std::net::SocketAddr,
        timestamp: time::Utc,
    ) {
        let this = self.0.lock().await;
        let mut validator_addrs = this.borrow().clone();
        let version = validator_addrs.get(&key.public()).map(|x| x.msg.version + 1).unwrap_or(0);
        let d = Arc::new(key.sign_msg(validator::NetAddress {
            addr,
            version,
            timestamp,
        }));
        validator_addrs.0.insert(d.key.clone(), d);
        this.send_replace(validator_addrs);
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
            this.send_replace(validator_addrs);
        }
        Ok(())
    }
}
