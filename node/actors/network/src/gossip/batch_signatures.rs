//! Global state distributed by active attesters, observed by all the nodes in the network.
use crate::watch::Watch;
use std::{collections::HashSet, sync::Arc};
use zksync_concurrency::sync;
use zksync_consensus_roles::attester::{self, Batch};

/// Mapping from attester::PublicKey to a signed attester::Batch message.
/// Represents the currents state of node's knowledge about the attester signatures.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct BatchSignatures(
    pub(super) im::HashMap<attester::PublicKey, Arc<attester::Signed<Batch>>>,
);

impl BatchSignatures {
    /// Returns a set of entries of `self` which are newer than the entries in `b`.
    pub(super) fn get_newer(&self, b: &Self) -> Vec<Arc<attester::Signed<Batch>>> {
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
        attesters: &attester::Committee,
        data: &[Arc<attester::Signed<Batch>>],
    ) -> anyhow::Result<bool> {
        let mut changed = false;

        let mut done = HashSet::new();
        for d in data {
            // Disallow multiple entries for the same key:
            // it is important because a malicious attester may spam us with
            // new versions and verifying signatures is expensive.
            if done.contains(&d.key) {
                anyhow::bail!("duplicate entry for {:?}", d.key);
            }
            done.insert(d.key.clone());
            if !attesters.contains(&d.key) {
                // We just skip the entries we are not interested in.
                // For now the set of attesters is static, so we could treat this as an error,
                // however we eventually want the attester set to be dynamic.
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

/// Watch wrapper of BatchSignatures,
/// which supports subscribing to BatchSignatures updates.
pub(crate) struct BatchSignaturesWatch(Watch<BatchSignatures>);

impl Default for BatchSignaturesWatch {
    fn default() -> Self {
        Self(Watch::new(BatchSignatures::default()))
    }
}

impl BatchSignaturesWatch {
    /// Subscribes to BatchSignatures updates.
    pub(crate) fn subscribe(&self) -> sync::watch::Receiver<BatchSignatures> {
        self.0.subscribe()
    }

    /// Inserts data to BatchSignatures.
    /// Subscribers are notified iff at least 1 new entry has
    /// been inserted. Returns an error iff an invalid
    /// entry in `data` has been found. The provider of the
    /// invalid entry should be banned.
    pub(crate) async fn update(
        &self,
        attesters: &attester::Committee,
        data: &[Arc<attester::Signed<Batch>>],
    ) -> anyhow::Result<()> {
        let this = self.0.lock().await;
        let mut signatures = this.borrow().clone();
        if signatures.update(attesters, data)? {
            this.send_replace(signatures);
        }
        Ok(())
    }
}
