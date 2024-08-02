use std::sync::Arc;
use std::cmp::Ordering;
use zksync_concurrency::{ctx,sync};
use zksync_consensus_roles::attester;
use std::collections::HashSet;
use anyhow::Context as _;
use crate::watch::Watch;

/// Coordinate the attestation by showing the config as seen by the main node.
#[derive(Debug, Clone)]
pub struct Config {
    pub batch_to_attest: attester::Batch,
    /// Committee for that batch.
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

impl State {
    /// Returns a set of votes of `self` which are newer than the entries in `old`.
    pub fn new_votes(&self, old: &Option<Self>) -> Vec<Arc<attester::Signed<attester::Batch>>> {
        let Some(old) = old.as_ref() else { return self.votes.values().cloned().collect() };
        match self.config.batch_to_attest.number.cmp(&old.config.batch_to_attest.number) {
            Ordering::Less => vec![],
            Ordering::Greater => self.votes.values().cloned().collect(),
            Ordering::Equal => self.votes.iter()
                .filter(|(k,_)|!old.votes.contains_key(k))
                .map(|(_,v)|v.clone())
                .collect()
        }
    }

    /// Verifies and adds a vote.
    /// Noop if vote is not signed by a committee memver or already inserted.
    /// Returns an error if genesis doesn't match or the signature is invalid.
    fn insert_vote(&mut self, vote: Arc<attester::Signed<attester::Batch>>) -> anyhow::Result<()> {
        anyhow::ensure!(vote.msg.genesis == self.config.batch_to_attest.genesis, "Genesis mismatch");
        if vote.msg != self.config.batch_to_attest { return Ok(()) }
        let Some(weight) = self.config.committee.weight(&vote.key) else { return Ok(()) };
        if self.votes.contains_key(&vote.key) { return Ok(()) }
        // Verify signature only after checking all the other preconditions. 
        vote.verify().context("verify")?;
        self.votes.insert(vote.key.clone(),vote);
        self.weight += weight;
        Ok(())
    }

    pub fn insert_votes(&mut self, votes: impl Iterator<Item=Arc<attester::Signed<attester::Batch>>>) -> anyhow::Result<()> {
        let mut done = HashSet::new();
        for vote in votes {
            // Disallow multiple entries for the same key:
            // it is important because a malicious attester may spam us with
            // new versions and verifying signatures is expensive.
            if done.contains(&vote.key) {
                anyhow::bail!("duplicate entry for {:?}", vote.key);
            }
            done.insert(vote.key.clone());
            self.insert_vote(vote);
        }
        Ok(())
    }
}

pub(crate) struct StateReceiver {
    prev: Option<State>,
    recv: sync::watch::Receiver<Option<State>>,
}

impl StateReceiver {
    pub async fn wait_for_new_votes(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<Vec<Arc<attester::Signed<attester::Batch>>>> {
        loop {
            let Some(new) = (*sync::changed(ctx, &mut self.recv).await?).clone() else { continue };
            let new_votes = new.new_votes(&self.prev);
            self.prev = Some(new);
            if !new_votes.is_empty() {
                return Ok(new_votes)
            }
        }
    }
}

pub struct StateWatch {
    key: Option<attester::SecretKey>,
    state: Watch<Option<State>>,
}

impl StateWatch {
    /// Constructs AttestationStatusWatch.
    pub fn new(key: Option<attester::SecretKey>) -> Self {
        Self { key, state: Watch::new(None) }
    }

    pub(crate) fn subscribe(&self) -> StateReceiver {
        let mut recv = self.state.subscribe();
        recv.mark_changed();
        StateReceiver { prev: None, recv }
    }

    pub async fn insert_votes(&self, votes: impl Iterator<Item=Arc<attester::Signed<attester::Batch>>>) -> anyhow::Result<()> {
        let locked = self.state.lock().await;
        let Some(mut state) = locked.borrow().clone() else { return Ok(()) };
        let before = state.weight;
        let res = state.insert_votes(votes);
        if state.weight > before {
            locked.send_replace(Some(state));
        }
        res
    }

    pub async fn wait_for_qc(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<attester::BatchQC> {
        sync::wait_for_some(ctx, &mut self.state.subscribe(), |state| {
            let state = state.as_ref()?;
            if state.weight < state.config.committee.threshold() {
                return None;
            }
            let mut sigs = attester::MultiSig::default();
            for vote in state.votes.values() {
                sigs.add(vote.key.clone(),vote.sig.clone());
            }
            Some(attester::BatchQC {
                message: state.config.batch_to_attest.clone(),
                signatures: sigs,
            })
        }).await
    }

    pub async fn set_config(&self, config: Config) -> anyhow::Result<()> {
        let locked = self.state.lock().await;
        let old = locked.borrow().clone();
        if let Some(old) = old.as_ref() {
            anyhow::ensure!(old.config.batch_to_attest.genesis == config.batch_to_attest.genesis, "tried to change genesis");
            anyhow::ensure!(old.config.batch_to_attest.number < config.batch_to_attest.number, "tried to decrease batch number");
        }
        let mut new = State { config: Arc::new(config), votes: im::HashMap::new(), weight: 0 };
        if let Some(key) = self.key.as_ref() {
            if new.config.committee.contains(&key.public()) {
                let vote = key.sign_msg(new.config.batch_to_attest.clone()); 
                // This is our own vote, so it always should be valid.
                new.insert_vote(Arc::new(vote)).unwrap();
            }
        }
        locked.send_replace(Some(new));
        Ok(())
    }
}
