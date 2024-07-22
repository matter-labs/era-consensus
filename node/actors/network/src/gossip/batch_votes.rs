//! Global state distributed by active attesters, observed by all the nodes in the network.
use crate::watch::Watch;
use std::{collections::HashSet, fmt, sync::Arc};
use zksync_concurrency::sync;
use zksync_consensus_roles::attester;

/// Represents the current state of node's knowledge about the attester votes.
///
/// Eventually this data structure will have to track voting potentially happening
/// simultaneously on multiple heights, if we decrease the batch interval to be
/// several seconds, instead of a minute. By that point, the replicas should be
/// following the main node (or L1) to know what is the highest finalized batch,
/// which will act as a floor to the batch number we have to track here. It will
/// also help to protect ourselves from DoS attacks by malicious attesters casting
/// votes far into the future.
///
/// For now, however, we just want a best effort where if we find a quorum, we
/// save it to the database, if not, we move on. For that, a simple protection
/// mechanism is to only allow one active vote per attester, which means any
/// previous vote can be removed when a new one is added.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct BatchVotes {
    /// The latest vote received from each attester. We only keep the last one
    /// for now, hoping that with 1 minute batches there's plenty of time for
    /// the quorum to be reached, but eventually we'll have to allow multiple
    /// votes across different heights.
    pub(crate) votes: im::HashMap<attester::PublicKey, Arc<attester::Signed<attester::Batch>>>,

    /// Total weight of votes at different heights and hashes.
    ///
    /// We will be looking for any hash that reaches a quorum threshold at any of the heights.
    /// At that point we can remove all earlier heights, considering it final. In the future
    /// we can instead keep heights until they are observed on the main node (or L1).
    pub(crate) support:
        im::OrdMap<attester::BatchNumber, im::HashMap<attester::BatchHash, attester::Weight>>,

    /// The minimum batch number for which we are still interested in votes.
    pub(crate) min_batch_number: attester::BatchNumber,
}

impl BatchVotes {
    /// Returns a set of votes of `self` which are newer than the entries in `b`.
    pub(super) fn get_newer(&self, b: &Self) -> Vec<Arc<attester::Signed<attester::Batch>>> {
        let mut newer = vec![];
        for (k, v) in &self.votes {
            if let Some(bv) = b.votes.get(k) {
                if v.msg <= bv.msg {
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
        data: &[Arc<attester::Signed<attester::Batch>>],
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

            if d.msg.number < self.min_batch_number {
                continue;
            }

            let Some(weight) = attesters.weight(&d.key) else {
                // We just skip the entries we are not interested in.
                // For now the set of attesters is static, so we could treat this as an error,
                // however we eventually want the attester set to be dynamic.
                continue;
            };

            // If we already have a newer vote for this key, we can ignore this one.
            if let Some(x) = self.votes.get(&d.key) {
                if d.msg <= x.msg {
                    continue;
                }
            }

            // Check the signature before insertion.
            d.verify()?;

            self.add(d.clone(), weight);

            changed = true;
        }
        Ok(changed)
    }

    /// Check if we have achieved quorum for any of the batch hashes.
    ///
    /// The return value is a vector because eventually we will be potentially waiting for
    /// quorums on multiple heights simultaneously.
    ///
    /// For repeated queries we can supply a skip list of heights for which we already saved the QC.
    pub(super) fn find_quorums(
        &self,
        attesters: &attester::Committee,
        skip: impl Fn(attester::BatchNumber) -> bool,
    ) -> Vec<attester::BatchQC> {
        let threshold = attesters.threshold();
        self.support
            .iter()
            .filter(|(number, _)| !skip(**number))
            .flat_map(|(number, candidates)| {
                candidates
                    .iter()
                    .filter(|(_, weight)| **weight >= threshold)
                    .map(|(hash, _)| {
                        let sigs = self
                            .votes
                            .values()
                            .filter(|vote| vote.msg.hash == *hash)
                            .map(|vote| (vote.key.clone(), vote.sig.clone()))
                            .fold(attester::MultiSig::default(), |mut sigs, (key, sig)| {
                                sigs.add(key, sig);
                                sigs
                            });
                        attester::BatchQC {
                            message: attester::Batch {
                                number: *number,
                                hash: *hash,
                            },
                            signatures: sigs,
                        }
                    })
            })
            .collect()
    }

    /// Set the minimum batch number for which we admit votes.
    ///
    /// Discards data about earlier heights.
    pub(super) fn set_min_batch_number(&mut self, min_batch_number: attester::BatchNumber) {
        self.min_batch_number = min_batch_number;
        self.votes.retain(|_, v| v.msg.number >= min_batch_number);
        if let Some(prev) = min_batch_number.prev() {
            self.support = self.support.split(&prev).1;
        }
    }

    /// Add an already validated vote from an attester into the register.
    fn add(&mut self, vote: Arc<attester::Signed<attester::Batch>>, weight: attester::Weight) {
        self.remove(&vote.key, weight);

        let batch = self.support.entry(vote.msg.number).or_default();
        let support = batch.entry(vote.msg.hash).or_default();
        *support = support.saturating_add(weight);

        self.votes.insert(vote.key.clone(), vote);
    }

    /// Remove any existing vote.
    ///
    /// This is for DoS protection, until we have better control over the acceptable vote range.
    fn remove(&mut self, key: &attester::PublicKey, weight: attester::Weight) {
        let Some(vote) = self.votes.remove(key) else {
            return;
        };

        let batch = self.support.entry(vote.msg.number).or_default();
        let support = batch.entry(vote.msg.hash).or_default();
        *support = support.saturating_sub(weight);

        if *support == 0u64 {
            batch.remove(&vote.msg.hash);
        }

        if batch.is_empty() {
            self.support.remove(&vote.msg.number);
        }
    }
}

/// Watch wrapper of BatchVotes,
/// which supports subscribing to BatchVotes updates.
pub(crate) struct BatchVotesWatch(Watch<BatchVotes>);

impl Default for BatchVotesWatch {
    fn default() -> Self {
        Self(Watch::new(BatchVotes::default()))
    }
}

impl BatchVotesWatch {
    /// Subscribes to BatchVotes updates.
    pub(crate) fn subscribe(&self) -> sync::watch::Receiver<BatchVotes> {
        self.0.subscribe()
    }

    /// Inserts data to BatchVotes.
    /// Subscribers are notified iff at least 1 new entry has
    /// been inserted. Returns an error iff an invalid
    /// entry in `data` has been found. The provider of the
    /// invalid entry should be banned.
    pub(crate) async fn update(
        &self,
        attesters: &attester::Committee,
        data: &[Arc<attester::Signed<attester::Batch>>],
    ) -> anyhow::Result<()> {
        let this = self.0.lock().await;
        let mut votes = this.borrow().clone();
        if votes.update(attesters, data)? {
            this.send_replace(votes);
        }
        Ok(())
    }

    /// Set the minimum batch number on the votes and discard old data.
    pub(crate) async fn set_min_batch_number(&self, min_batch_number: attester::BatchNumber) {
        let this = self.0.lock().await;
        this.send_modify(|votes| votes.set_min_batch_number(min_batch_number));
    }
}

/// Wrapper around [BatchVotesWatch] to publish votes over batches signed by an attester key.
pub struct BatchVotesPublisher(pub(crate) Arc<BatchVotesWatch>);

impl fmt::Debug for BatchVotesPublisher {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("BatchVotesPublisher")
            .finish_non_exhaustive()
    }
}

impl BatchVotesPublisher {
    /// Sign an L1 batch and push it into the batch, which should cause it to be gossiped by the network.
    pub async fn publish(
        &self,
        attesters: &attester::Committee,
        attester: &attester::SecretKey,
        batch: attester::Batch,
    ) -> anyhow::Result<()> {
        if !attesters.contains(&attester.public()) {
            return Ok(());
        }
        let attestation = attester.sign_msg(batch);
        self.0.update(attesters, &[Arc::new(attestation)]).await
    }
}
