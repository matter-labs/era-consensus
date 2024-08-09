//! Attestation.
use crate::watch::Watch;
use anyhow::Context as _;
use std::{cmp::Ordering, collections::HashSet, fmt, sync::Arc};
use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::attester;

mod metrics;
#[cfg(test)]
mod tests;

/// Coordinate the attestation by showing the config as seen by the main node.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Batch to attest.
    pub batch_to_attest: attester::Batch,
    /// Committee that should attest the batch.
    /// NOTE: the committee is not supposed to change often,
    /// so you might want to use `Arc<attester::Committee>` instead
    /// to avoid extra copying.
    pub committee: attester::Committee,
}

/// A [Watch] over an [AttestationStatus] which we can use to notify components about
/// changes in the batch number the main node expects attesters to vote on.
#[derive(Clone)]
pub(crate) struct State {
    config: Arc<Config>,
    votes: im::HashMap<attester::PublicKey, Arc<attester::Signed<attester::Batch>>>,
    weight: attester::Weight,
}

/// Diff between 2 states.
pub(crate) struct Diff {
    /// New votes.
    pub(crate) votes: Vec<Arc<attester::Signed<attester::Batch>>>,
    /// Whether the config has changed.
    pub(crate) config_changed: bool,
}

impl Diff {
    fn is_empty(&self) -> bool {
        self.votes.is_empty() && !self.config_changed
    }
}

impl State {
    /// Returns a set of votes of `self` which are newer than the entries in `old`.
    fn diff(&self, old: &Option<Self>) -> Diff {
        let Some(old) = old.as_ref() else {
            return Diff {
                config_changed: true,
                votes: self.votes.values().cloned().collect(),
            };
        };
        match self
            .config
            .batch_to_attest
            .number
            .cmp(&old.config.batch_to_attest.number)
        {
            Ordering::Less => Diff {
                config_changed: true,
                votes: vec![],
            },
            Ordering::Greater => Diff {
                config_changed: true,
                votes: self.votes.values().cloned().collect(),
            },
            Ordering::Equal => Diff {
                config_changed: false,
                votes: self
                    .votes
                    .iter()
                    .filter(|(k, _)| !old.votes.contains_key(k))
                    .map(|(_, v)| v.clone())
                    .collect(),
            },
        }
    }

    /// Verifies and adds a vote.
    /// Noop if vote is not signed by a committee memver or already inserted.
    /// Returns an error if genesis doesn't match or the signature is invalid.
    fn insert_vote(&mut self, vote: Arc<attester::Signed<attester::Batch>>) -> anyhow::Result<()> {
        anyhow::ensure!(
            vote.msg.genesis == self.config.batch_to_attest.genesis,
            "Genesis mismatch"
        );
        if vote.msg != self.config.batch_to_attest {
            return Ok(());
        }
        let Some(weight) = self.config.committee.weight(&vote.key) else {
            return Ok(());
        };
        if self.votes.contains_key(&vote.key) {
            return Ok(());
        }
        // Verify signature only after checking all the other preconditions.
        vote.verify().context("verify")?;
        self.votes.insert(vote.key.clone(), vote);
        self.weight += weight;
        Ok(())
    }

    fn insert_votes(
        &mut self,
        votes: impl Iterator<Item = Arc<attester::Signed<attester::Batch>>>,
    ) -> anyhow::Result<()> {
        let mut done = HashSet::new();
        for vote in votes {
            // Disallow multiple entries for the same key:
            // it is important because a malicious attester may spam us with
            // new versions and verifying signatures is expensive.
            if done.contains(&vote.key) {
                anyhow::bail!("duplicate entry for {:?}", vote.key);
            }
            done.insert(vote.key.clone());
            self.insert_vote(vote)?;
        }
        Ok(())
    }

    fn qc(&self) -> Option<attester::BatchQC> {
        if self.weight < self.config.committee.threshold() {
            return None;
        }
        let mut sigs = attester::MultiSig::default();
        for vote in self.votes.values() {
            sigs.add(vote.key.clone(), vote.sig.clone());
        }
        Some(attester::BatchQC {
            message: self.config.batch_to_attest.clone(),
            signatures: sigs,
        })
    }
}

/// Received of state diffs.
pub(crate) struct DiffReceiver {
    prev: Option<State>,
    recv: sync::watch::Receiver<Option<State>>,
}

impl DiffReceiver {
    /// Waits for the next state diff.
    pub(crate) async fn wait_for_diff(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<Diff> {
        loop {
            let Some(new) = (*sync::changed(ctx, &mut self.recv).await?).clone() else {
                continue;
            };
            let diff = new.diff(&self.prev);
            self.prev = Some(new);
            if !diff.is_empty() {
                return Ok(diff);
            }
        }
    }
}

/// Watch of the attestation state.
pub struct StateWatch {
    key: Option<attester::SecretKey>,
    state: Watch<Option<State>>,
}

impl fmt::Debug for StateWatch {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("StateWatch").finish_non_exhaustive()
    }
}

impl StateWatch {
    /// Constructs AttestationStatusWatch.
    pub fn new(key: Option<attester::SecretKey>) -> Self {
        Self {
            key,
            state: Watch::new(None),
        }
    }

    /// Subscribes to state diffs.
    pub(crate) fn subscribe(&self) -> DiffReceiver {
        let mut recv = self.state.subscribe();
        recv.mark_changed();
        DiffReceiver { prev: None, recv }
    }

    /// Inserts votes to the state.
    /// Irrelevant votes are silently ignored.
    /// Returns an error if an invalid vote has been found.
    /// It is possible that the state has been updated even if an error
    /// was returned.
    pub(crate) async fn insert_votes(
        &self,
        votes: impl Iterator<Item = Arc<attester::Signed<attester::Batch>>>,
    ) -> anyhow::Result<()> {
        let locked = self.state.lock().await;
        let Some(mut state) = locked.borrow().clone() else {
            return Ok(());
        };
        let before = state.weight;
        let res = state.insert_votes(votes);
        if state.weight > before {
            metrics::METRICS.votes_collected.set(state.votes.len());
            metrics::METRICS
                .weight_collected
                .set(state.weight as f64 / state.config.committee.total_weight() as f64);
            locked.send_replace(Some(state));
        }
        res
    }

    /// All votes kept in the state.
    pub(crate) fn votes(&self) -> Vec<Arc<attester::Signed<attester::Batch>>> {
        self.state
            .subscribe()
            .borrow()
            .iter()
            .flat_map(|s| s.votes.values().cloned())
            .collect()
    }

    /// Waits for the certificate for a batch with the given number to be collected.
    /// Returns None iff attestation already skipped to collecting certificate some later batch.
    pub async fn wait_for_qc(
        &self,
        ctx: &ctx::Ctx,
        n: attester::BatchNumber,
    ) -> ctx::OrCanceled<Option<attester::BatchQC>> {
        let recv = &mut self.state.subscribe();
        recv.mark_changed();
        loop {
            let state = sync::changed(ctx, recv).await?;
            let Some(state) = state.as_ref() else {
                continue;
            };
            if state.config.batch_to_attest.number < n {
                continue;
            };
            if state.config.batch_to_attest.number > n {
                return Ok(None);
            }
            if let Some(qc) = state.qc() {
                return Ok(Some(qc));
            }
        }
    }

    /// Updates the attestation config.
    /// Clears the votes collected for the previous config.
    /// Batch number has to increase with each update.
    pub async fn update_config(&self, config: Config) -> anyhow::Result<()> {
        let locked = self.state.lock().await;
        let old = locked.borrow().clone();
        if let Some(old) = old.as_ref() {
            if *old.config == config {
                return Ok(());
            }
            anyhow::ensure!(
                old.config.batch_to_attest.genesis == config.batch_to_attest.genesis,
                "tried to change genesis"
            );
            anyhow::ensure!(
                old.config.batch_to_attest.number < config.batch_to_attest.number,
                "tried to decrease batch number"
            );
        }
        let mut new = State {
            config: Arc::new(config),
            votes: im::HashMap::new(),
            weight: 0,
        };
        if let Some(key) = self.key.as_ref() {
            if new.config.committee.contains(&key.public()) {
                let vote = key.sign_msg(new.config.batch_to_attest.clone());
                // This is our own vote, so it always should be valid.
                new.insert_vote(Arc::new(vote)).unwrap();
            }
        }
        metrics::METRICS
            .batch_number
            .set(new.config.batch_to_attest.number.0);
        metrics::METRICS
            .committee_size
            .set(new.config.committee.len());
        metrics::METRICS.votes_collected.set(new.votes.len());
        metrics::METRICS
            .weight_collected
            .set(new.weight as f64 / new.config.committee.total_weight() as f64);
        locked.send_replace(Some(new));
        Ok(())
    }
}
