//! Attestation.
use crate::watch::Watch;
use anyhow::Context as _;
use std::{collections::HashSet, fmt, sync::Arc};
use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::attester;

mod metrics;
#[cfg(test)]
mod tests;

/// Configuration of the attestation Controller.
/// It determines what should be attested and by whom.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Batch to attest.
    pub batch_to_attest: attester::Batch,
    /// Committee that should attest the batch.
    pub committee: Arc<attester::Committee>,
}

// Internal attestation state: config and the set of votes collected so far.
#[derive(Clone)]
struct State {
    config: Arc<Config>,
    /// Votes collected so far.
    votes: im::HashMap<attester::PublicKey, Arc<attester::Signed<attester::Batch>>>,
    // Total weight of the votes collected.
    total_weight: attester::Weight,
}

/// Diff between 2 states.
pub(crate) struct Diff {
    /// New votes.
    pub(crate) votes: Vec<Arc<attester::Signed<attester::Batch>>>,
    /// New config, if changed.
    pub(crate) config: Option<Arc<Config>>,
}

impl Diff {
    fn is_empty(&self) -> bool {
        self.votes.is_empty() && self.config.is_none()
    }
}

impl State {
    /// Returns a diff between `self` state and `old` state.
    /// Diff contains votes which are present is `self`, but not in `old`.
    fn diff(&self, old: &Option<Self>) -> Diff {
        let Some(old) = old.as_ref() else {
            return Diff {
                config: Some(self.config.clone()),
                votes: self.votes.values().cloned().collect(),
            };
        };
        if self.config.batch_to_attest.number != old.config.batch_to_attest.number {
            return Diff {
                config: Some(self.config.clone()),
                votes: self.votes.values().cloned().collect(),
            };
        }

        Diff {
            config: None,
            votes: self
                .votes
                .iter()
                .filter(|(k, _)| !old.votes.contains_key(k))
                .map(|(_, v)| v.clone())
                .collect(),
        }
    }

    /// Verifies and adds a vote.
    /// Noop if vote is not signed by a committee member or already inserted.
    /// Returns an error if genesis doesn't match or the signature is invalid.
    fn insert_vote(&mut self, vote: Arc<attester::Signed<attester::Batch>>) -> anyhow::Result<()> {
        anyhow::ensure!(
            vote.msg.genesis == self.config.batch_to_attest.genesis,
            "Genesis mismatch"
        );
        if vote.msg.number != self.config.batch_to_attest.number {
            return Ok(());
        }
        anyhow::ensure!(
            vote.msg.hash == self.config.batch_to_attest.hash,
            "batch hash mismatch"
        );
        let Some(weight) = self.config.committee.weight(&vote.key) else {
            anyhow::bail!(
                "received vote signed by an inactive attester: {:?}",
                vote.key
            );
        };
        if self.votes.contains_key(&vote.key) {
            return Ok(());
        }
        // Verify signature only after checking all the other preconditions.
        vote.verify().context("verify")?;
        tracing::info!("collected vote with weight {weight} from {:?}", vote.key);
        self.votes.insert(vote.key.clone(), vote);
        self.total_weight += weight;
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

    fn cert(&self) -> Option<attester::BatchQC> {
        if self.total_weight < self.config.committee.threshold() {
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

/// Receiver of state diffs.
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

/// `Controller` manages the attestation state.
/// It maintains a set of votes matching the attestation config.
/// It allows for
/// * adding votes to the state
/// * subscribing to the vote set changes
/// * waiting for the certificate to be collected
///
/// It also keeps an attester key used to sign the batch vote,
/// whenever it belongs the current attester committee.
/// Signing happens automatically whenever the committee is updated.
///
/// Expected usage:
/// ```
/// let ctrl = Arc::new(attestation::Controller::new(Some(key)));
/// // Check what is the number of the next batch to be attested in a
/// // global attestation registry (i.e. L1 chain state).
/// let first : attester::BatchNumber = ...
/// scope::run!(ctx, |ctx,s| async {
///     // Loop starting attestation whenever global attestation state progresses.
///     s.spawn(async {
///         let mut next = first;
///         loop {
///             // Based on the local storage, compute the next expected batch hash
///             // and the committee that should attest it.
///             ...
///             let config = attestation::Config {
///                 batch_to_attest: attester::Batch {
///                     number: next,
///                     ...
///                 },
///                 committee: ...,
///             };
///             ctrl.start_attestation(Arc::new(config)).unwrap();
///             // Wait for the attestation to progress, by observing the
///             // global attestation registry.
///             next = ...;
///         }
///     });
///     s.spawn(async {
///         // Loop waiting for a certificate to be collected and submitting
///         // it to the global registry
///         loop {
///             let mut next = first;
///             if let Some(qc) = ctrl.wait_for_cert(ctx, next).await?;
///             // Submit the certificate to the global registry.
///             ...
///             next = next.next();
///         }
///     });
///
///     // Make the executor establish the p2p network and
///     // collect the attestation votes.
///     executor::Executor {
///         ...
///         attestation: ctrl.clone(),
///     }.run(ctx).await;
/// }
/// ```
pub struct Controller {
    /// Key to automatically vote for batches.
    /// None, if the current node is not an attester.
    key: Option<attester::SecretKey>,
    /// Internal state of the controller.
    state: Watch<Option<State>>,
}

impl fmt::Debug for Controller {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("StateWatch")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl Controller {
    /// Constructs Controller.
    /// `key` will be used for automatically signing votes.
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
    /// It is possible that some votes have been added to the state
    /// even if eventually an error was returned.
    pub(crate) async fn insert_votes(
        &self,
        votes: impl Iterator<Item = Arc<attester::Signed<attester::Batch>>>,
    ) -> anyhow::Result<()> {
        let locked = self.state.lock().await;
        let Some(mut state) = locked.borrow().clone() else {
            return Ok(());
        };
        let before = state.total_weight;
        let res = state.insert_votes(votes);
        if state.total_weight > before {
            metrics::METRICS.votes_collected.set(state.votes.len());
            #[allow(clippy::float_arithmetic)]
            metrics::METRICS
                .weight_collected
                .set(state.total_weight as f64 / state.config.committee.total_weight() as f64);
            locked.send_replace(Some(state));
        }
        res
    }

    /// Returns votes matching the `want` batch.
    pub(crate) fn votes(
        &self,
        want: &attester::Batch,
    ) -> Vec<Arc<attester::Signed<attester::Batch>>> {
        let state = self.state.subscribe();
        let state = state.borrow();
        let Some(state) = &*state else { return vec![] };
        if &state.config.batch_to_attest != want {
            return vec![];
        }
        state.votes.values().cloned().collect()
    }

    /// Waits for the certificate for a batch with the given number to be collected.
    /// Returns None iff attestation already skipped to collecting certificate for some later batch.
    pub async fn wait_for_cert(
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
            if let Some(qc) = state.cert() {
                return Ok(Some(qc));
            }
        }
    }

    /// Updates the attestation config.
    /// Clears the votes collected for the previous config.
    /// Batch number has to increase with each update.
    #[tracing::instrument(name = "attestation::Controller::start_attestation", skip_all)]
    pub async fn start_attestation(&self, config: Arc<Config>) -> anyhow::Result<()> {
        let locked = self.state.lock().await;
        let old = locked.borrow().clone();
        if let Some(old) = old.as_ref() {
            if *old.config == *config {
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
        tracing::info!(
            "started collecting votes for batch {:?}",
            config.batch_to_attest.number
        );
        let mut new = State {
            config,
            votes: im::HashMap::new(),
            total_weight: 0,
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
        #[allow(clippy::float_arithmetic)]
        metrics::METRICS
            .weight_collected
            .set(new.total_weight as f64 / new.config.committee.total_weight() as f64);
        locked.send_replace(Some(new));
        Ok(())
    }
}
