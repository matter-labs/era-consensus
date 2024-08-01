use std::fmt;
use std::sync::Arc;
use zksync_concurrency::sync;
use zksync_consensus_roles::{attester};

use crate::watch::Watch;

/// Coordinate the attestation by showing the status as seen by the main node.
#[derive(Debug, Clone)]
pub struct AttestationStatus {
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
    status: Arc<AttestationStatus>,
    votes: im::HashMap<attester::PublicKey, Arc<attester::Signed<attester::Batch>>>,
    weight: attester::Weight,
}

impl Inner {
    /// Verifies and adds a vote.
    fn insert_vote(&mut self, vote: Arc<attester::Signed<attester::Batch>>) -> anyhow::Result<()> {
        anyhow::ensure(vote.msg.genesis == self.status.batch_to_attest.genesis, "Genesis mismatch");
        if vote.msg != self.status.batch_to_attest { return Ok(()) }
        let Some(weight) = self.status.committee.contains(&vote.key) else { return Ok(()) };
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
            done.insert(&vote.key);
            self.insert_vote(vote);
        }
        Ok(())
    }
}

struct AttestationStatusWatch(watch::Sender<Inner>);

impl AttestationStatusWatch {
    /// Constructs AttestationStatusWatch.
    pub fn new(s: AttestationStatus) -> Self {
        Self(Watch::new(Inner {
            status: s.into(),
            votes: [].into(),
            weight: 0,
        }))
    }

    pub fn insert_votes(&mut self, votes: impl Iterator<Arc<attester::Signer<attester::Batch>>>) -> anyhow::Result<()> {
        let mut res;
        self.0.send_if_modified(|this| {
            let before = this.weight;
            res = this.insert_votes(votes);
            this.weight > before
        });
        res
    }

    pub async fn wait_for_qc(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<attester::BatchQC> {
        let sub = &mut self.0.subscribe();
        let t = sub.borrow().status.committee.threshold();
        let this = sync::wait_for(ctx, sub, |this| this.weight >= t).await?;
        let mut sigs = attester::MultiSig::default();
        for vote in this.votes.values() {
            sigs.add(vote.key.clone(),vote.sig.clone());
        }
        Ok(attester::BatchQC {
            message: this.status.batch_to_attest.clone(),
            signatures: sigs,
        })
    }

    /// Subscribes to AttestationStatus updates.
    pub fn subscribe(&self) -> AttestationStatusReceiver {
        self.0.subscribe()
    } 

    /// Set the next batch number to attest on and notify subscribers it changed.
    /// Returns an error if the update it not valid.
    pub fn update(&self, s: AttestationStatus) -> anyhow::Result<()> {
        sync::try_send_modify(&self.0, |this| {
            anyhow::ensure!(this.status.batch_to_attest.genesis == s.batch_to_attest.genesis, "tried to change genesis");
            anyhow::ensure!(this.status.batch_to_attest.message.number < s.batch_to_attest.message.number, "tried to decrease batch number");
            this.status = s;
            this.votes.clear();
            this.weight = 0;
            Ok(())
        })
    }
}
