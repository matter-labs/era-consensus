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
    status: Arc<AttestationStatusWatch>,
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
    pub(super) async fn update(
        &mut self,
        votes: &[Arc<attester::Signed<attester::Batch>>],
    ) -> anyhow::Result<BatchUpdateStats> {
        let mut stats = BatchUpdateStats::default();
        votes. 
        Ok(stats)
    }

    /// Discards data about earlier heights.
    pub(super) fn set_status(&mut self, status: Arc<AttestationStatus>) {
        self.votes.clear();
        self.partial_qc = attester::BatchQC::new(status.batch_to_attest.clone());
        self.status = status;
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
