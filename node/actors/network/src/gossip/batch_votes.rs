//! Global state distributed by active attesters, observed by all the nodes in the network.
use super::metrics;
use crate::watch::Watch;
use crate::gossip::AttestationStatus;
use std::cmp::Ordering;
use std::{collections::HashSet, fmt, sync::Arc};
use zksync_concurrency::sync;
use zksync_consensus_roles::attester;

#[derive(Debug, Default)]
pub(super) struct BatchUpdateStats {
    num_added: usize,
    weight_added: u64,
    last_added: Option<attester::BatchNumber>,
}

impl BatchUpdateStats {
    fn added(&mut self, number: attester::BatchNumber, weight: u64) {
        self.num_added += 1;
        self.weight_added += weight;
        self.last_added = Some(number);
    }
}

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
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BatchVotes {
    status: Arc<AttestationStatus>,
    votes: im::HashMap<attester::PublicKey, Arc<attester::Signed<attester::Batch>>>,
    partial_qc: attester::BatchQC,
}

impl BatchVotes {
    fn new(status: Arc<AttestationStatus>) -> Self {
        Self { status, votes: [].into(), partial_qcs: [].into() }
    }

    /// Returns a set of votes of `self` which are newer than the entries in `old`.
    pub(super) fn get_newer(&self, old: &Self) -> Vec<Arc<attester::Signed<attester::Batch>>> {
        match self.status.batch_to_attest.number.cmp(old.status.batch_to_attest.number) {
            Ordering::Less => vec![],
            Ordering::Greater => self.votes.values().cloned().collect(),
            Ordering::Equal => self.votes.iter()
                .filter(|(k,_)|!old.contains(k))
                .map(|(_,v)|v.clone())
                .collect()
        }
    }

    /// Updates the discovery map with entries from `data`.
    /// It exits as soon as an invalid entry is found.
    /// `self` might get modified even if an error is returned
    /// (all entries verified so far are added).
    ///
    /// Returns statistics about new entries added.
    pub(super) fn update(
        &mut self,
        votes: &[Arc<attester::Signed<attester::Batch>>],
    ) -> anyhow::Result<BatchUpdateStats> {
        let mut stats = BatchUpdateStats::default();
        let mut done = HashSet::new();
        for vote in votes {
            // Disallow multiple entries for the same key:
            // it is important because a malicious attester may spam us with
            // new versions and verifying signatures is expensive.
            if done.contains(&vote.key) {
                anyhow::bail!("duplicate entry for {:?}", vote.key);
            }
            done.insert(vote.key.clone());
            self.add(vote, &mut stats)?;
        }
        Ok(stats)
    }

    /// Check if we have achieved quorum for the current batch number.
    pub(super) fn find_quorum(&self) -> Option<attester::BatchQC> {
        match self.partial_qc.verify() {
            Ok(()) => Some(self.partial_qc.clone()),
            Err(_) => None
        }
    }

    /// Discards data about earlier heights.
    pub(super) fn set_status(&mut self, status: Arc<AttestationStatus>) {
        self.votes.clear();
        self.partial_qc = attester::BatchQC::new(status.batch_to_attest.clone());
        self.status = status;
    }

    /// Verifies and adds a vote.
    fn add(&mut self, vote: Arc<attester::Signed<attester::Batch>>, stats: &mut BatchUpdateStats) -> anyhow::Result<()> {
        // Genesis has to match 
        anyhow::ensure!(
            vote.msg.genesis == self.status.genesis,
            "vote for batch with different genesis hash: {:?}",
            vote.msg.genesis
        );

        // Skip the signatures for the irrelevant batch.
        if vote.message != self.status.batch_to_attest {
            return Ok(());
        }

        // We just skip the entries we are not interested in.
        let Some(weight) = self.status.committee.weight(&vote.key) else {
            return Ok(());
        };

        // If we already have a newer vote for this key, we can ignore this one.
        if self.votes.contains(&vote.key) { return Ok(()) }

        // Check the signature before insertion.
        vote.verify().context("verify()")?;
       
        // Insert the vote.
        stats.added(vote.msg.number, weight);
        self.partial_qc.signatures.add(vote.key, vote.sig);
        Ok(())
    }
}

/// Watch wrapper of BatchVotes,
/// which supports subscribing to BatchVotes updates.
pub(crate) struct BatchVotesWatch(Watch<BatchVotes>);

impl BatchVotesWatch {
    pub(crate) fn new(status: AttestationStatus) -> Self {
        Self(Watch::new(BatchVotes::new(status)))
    }

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
        data: &[Arc<attester::Signed<attester::Batch>>],
    ) -> anyhow::Result<()> {
        let this = self.0.lock().await;
        let mut votes = this.borrow().clone();
        let stats = votes.update(data)?;

        if let Some(last_added) = stats.last_added {
            this.send_replace(votes);

            #[allow(clippy::float_arithmetic)]
            let weight_added = stats.weight_added as f64 / votes.status.committee.total_weight() as f64;

            metrics::BATCH_VOTES_METRICS
                .last_added_vote_batch_number
                .set(last_added.0);

            metrics::BATCH_VOTES_METRICS
                .votes_added
                .inc_by(stats.num_added as u64);

            metrics::BATCH_VOTES_METRICS
                .weight_added
                .inc_by(weight_added);
        }
        Ok(())
    }

    /// Set the minimum batch number on the votes and discard old data.
    pub(crate) async fn set_status(&self, status: Arc<AttestationStatus>) {
        metrics::BATCH_VOTES_METRICS.min_batch_number.set(status.next_batch_to_attest.0);
        let this = self.0.lock().await;
        this.send_modify(|votes| votes.set_status(status));
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
    pub async fn publish(&self, attester: &attester::SecretKey) -> anyhow::Result<()> {
        if !self.committee.contains(&attester.public()) {
            return Ok(());
        }
        let attestation = attester.sign_msg(self.batch_to_attest);

        metrics::BATCH_VOTES_METRICS
            .last_signed_batch_number
            .set(attestation.msg.number.0);

        self.0.update(&[Arc::new(attestation)]).await
    }
}
