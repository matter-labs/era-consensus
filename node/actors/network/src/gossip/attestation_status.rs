use std::fmt;
use std::sync::Arc;
use zksync_concurrency::sync;
use zksync_consensus_roles::{attester};
use std::collections::HashSet;

use crate::watch::Watch;

/// Coordinate the attestation by showing the config as seen by the main node.
#[derive(Debug, Clone)]
pub struct AttestationConfig {
    pub batch_to_attest: attester::Batch,
    /// Committee for that batch.
    /// NOTE: the committee is not supposed to change often,
    /// so you might want to use `Arc<attester::Committee>` instead
    /// to avoid extra copying.
    pub committee: attester::Committee, 
}

/// A [Watch] over an [AttestationStatus] which we can use to notify components about
/// changes in the batch number the main node expects attesters to vote on.
struct Inner {
    config: Arc<AttestationConfig>,
    votes: im::HashMap<attester::PublicKey, Arc<attester::Signed<attester::Batch>>>,
    weight: attester::Weight,
}

impl Inner {
    /// Returns a set of votes of `self` which are newer than the entries in `old`.
    fn new_votes(&self, old: &Self) -> Vec<Arc<attester::Signed<attester::Batch>>> {
        match self.config.batch_to_attest.number.cmp(old.config.batch_to_attest.number) {
            Ordering::Less => vec![],
            Ordering::Greater => self.votes.values().cloned().collect(),
            Ordering::Equal => self.votes.iter()
                .filter(|(k,_)|!old.contains(k))
                .map(|(_,v)|v.clone())
                .collect()
        }
    }

    /// Verifies and adds a vote.
    fn insert_vote(&mut self, vote: Arc<attester::Signed<attester::Batch>>) -> anyhow::Result<()> {
        anyhow::ensure(vote.msg.genesis == self.config.batch_to_attest.genesis, "Genesis mismatch");
        if vote.msg != self.config.batch_to_attest { return Ok(()) }
        let Some(weight) = self.config.committee.contains(&vote.key) else { return Ok(()) };
        if self.votes.contains(&vote.key) { return Ok(()) }
        vote.verify().context("verify")?;
        self.votes.insert(vote.key.clone(),vote);
        self.weight += weight;
        Ok(())
    }

    pub fn insert_votes(&mut self, votes: impl Iterator<Arc<attester::Signer<attester::Batch>>>) -> anyhow::Result<()> {
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

pub struct AttestationState {
    key: Option<attester::SecretKey>,
    inner: Watch<Inner>,
}

impl AttestationState {
    /// Constructs AttestationStatusWatch.
    pub fn new(s: AttestationConfig, key: Option<attester::SecretKey>) -> Self {
        Self {
            key,
            inner: Watch::new(Inner {
                config: s.into(),
                votes: [].into(),
                weight: 0,
            }),
        }
    }

    pub async fn insert_votes(&mut self, votes: impl Iterator<Arc<attester::Signer<attester::Batch>>>) -> anyhow::Result<()> {
        let this = self.inner.lock().await;
        let mut inner = this.borrow().clone();
        let before = inner.weight;
        let res = inner.insert_votes(votes);
        if inner.weight > before {
            this.send_replace(inner);
        }
        res
    }

    pub async fn wait_for_qc(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<attester::BatchQC> {
        let sub = &mut self.inner.subscribe();
        let t = sub.borrow().config.committee.threshold();
        let this = sync::wait_for(ctx, sub, |this| this.weight >= t).await?;
        let mut sigs = attester::MultiSig::default();
        for vote in this.votes.values() {
            sigs.add(vote.key.clone(),vote.sig.clone());
        }
        Ok(attester::BatchQC {
            message: this.config.batch_to_attest.clone(),
            signatures: sigs,
        })
    }

    pub async fn advance(&self, s: AttestationConfig) -> anyhow::Result<()> {
        let vote = self.key.as_ref().map(|key| key.sign_msg(s.batch_to_attest.clone().insert())); 
        sync::try_send_modify(&self.inner.lock().await, |this| {
            anyhow::ensure!(this.config.batch_to_attest.genesis == s.batch_to_attest.genesis, "tried to change genesis");
            anyhow::ensure!(this.config.batch_to_attest.message.number < s.batch_to_attest.message.number, "tried to decrease batch number");
            this.config = Arc::new(s);
            this.votes.clear();
            this.weight = 0;
            if let Some(vote) = vote {
                this.insert_vote(Arc::new(vote)).unwrap();
            }
            Ok(())
        })
    }
}
