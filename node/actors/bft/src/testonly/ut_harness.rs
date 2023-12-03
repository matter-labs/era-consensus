use crate::{
    io::{InputMessage, OutputMessage},
    leader::{ReplicaCommitError, ReplicaPrepareError},
    replica::{LeaderPrepareError, LeaderCommitError},
    Consensus,
};
use zksync_consensus_utils::enum_util::{Variant};
use assert_matches::assert_matches;
use rand::{Rng};
use std::cmp::Ordering;
use zksync_concurrency::ctx;
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator::{
    self, BlockHeader, CommitQC, LeaderCommit, LeaderPrepare, Payload, Phase,
    PrepareQC, ReplicaCommit, ReplicaPrepare, SecretKey, Signed, ViewNumber,
};
use zksync_consensus_utils::pipe::DispatcherPipe;

/// `UTHarness` provides various utilities for unit tests.
/// It is designed to simplify the setup and execution of test cases by encapsulating
/// common testing functionality.
///
/// It should be instantiated once for every test case.
#[cfg(test)]
pub(crate) struct UTHarness {
    pub consensus: Consensus,
    pipe: DispatcherPipe<InputMessage, OutputMessage>,
    keys: Vec<SecretKey>,
}

impl UTHarness {
    /// Creates a new `UTHarness` with minimally-significant validator set size.
    pub(crate) async fn new_many(ctx: &ctx::Ctx) -> UTHarness {
        let num_validators = 6;
        assert!(crate::misc::faulty_replicas(num_validators) > 0);
        UTHarness::new(ctx,num_validators).await
    }

    /// Creates a new `UTHarness` with the specified validator set size.
    pub(crate) async fn new(ctx: &ctx::Ctx, num_validators: usize) -> UTHarness {
        let mut rng = ctx.rng();
        let keys: Vec<_> = (0..num_validators).map(|_| rng.gen()).collect();
        let (genesis, val_set) = crate::testonly::make_genesis(
            &keys,
            validator::ProtocolVersion::EARLIEST,
            Payload(vec![]),
        );
        let (mut consensus, pipe) =
            crate::testonly::make_consensus(&ctx, &keys[0], &val_set, &genesis).await;

        consensus.leader.view = ViewNumber(1);
        consensus.replica.view = ViewNumber(1);
        UTHarness { consensus, pipe, keys }
    }

    pub(crate) fn consensus_threshold(&self) -> usize {
        crate::misc::consensus_threshold(self.keys.len())
    }

    pub(crate) fn owner_key(&self) -> &SecretKey {
        &self.consensus.inner.secret_key
    }

    pub(crate) fn owner_as_view_leader(&self) -> ViewNumber {
        let mut view = self.current_replica_view();
        while self.view_leader(view) != self.owner_key().public() {
            view = view.next();
        }
        view
    }

    pub(crate) fn key_at(&self, index: usize) -> &SecretKey {
        &self.keys[index]
    }

    pub(crate) fn keys(&self) -> Vec<SecretKey> {
        self.keys.clone()
    }

    pub(crate) fn set_view(&mut self, view: ViewNumber) {
        self.set_replica_view(view);
        self.set_leader_view(view);
    }

    pub(crate) fn set_leader_view(&mut self, view: ViewNumber) {
        self.consensus.leader.view = view
    }

    pub(crate) fn set_leader_phase(&mut self, phase: Phase) {
        self.consensus.leader.phase = phase
    }

    pub(crate) fn set_replica_view(&mut self, view: ViewNumber) {
        self.consensus.replica.view = view
    }

    pub(crate) fn new_unfinalized_replica_prepare(&self) -> Signed<ReplicaPrepare> {
        self.new_current_replica_prepare(|msg| {
            let mut high_vote = ReplicaCommit {
                protocol_version: validator::ProtocolVersion::EARLIEST,
                view: self.consensus.replica.view.next(),
                proposal: self.consensus.replica.high_qc.message.proposal,
            };

            high_vote.proposal.parent = high_vote.proposal.hash();
            high_vote.proposal.number = high_vote.proposal.number.next();

            msg.high_vote = high_vote;
        })
    }

    pub(crate) fn new_current_replica_prepare(
        &self,
        mutate_fn: impl FnOnce(&mut ReplicaPrepare),
    ) -> Signed<ReplicaPrepare> {
        let mut msg = ReplicaPrepare {
            protocol_version: validator::ProtocolVersion::EARLIEST,
            view: self.consensus.replica.view,
            high_vote: self.consensus.replica.high_vote,
            high_qc: self.consensus.replica.high_qc.clone(),
        };
        mutate_fn(&mut msg);
        self.consensus.inner.secret_key.sign_msg(msg)
    }

    pub(crate) fn new_rnd_leader_prepare(
        &mut self,
        rng: &mut impl Rng,
        mutate_fn: impl FnOnce(&mut LeaderPrepare),
    ) -> Signed<LeaderPrepare> {
        let payload: Payload = rng.gen();
        let mut msg = LeaderPrepare {
            protocol_version: validator::ProtocolVersion::EARLIEST,
            view: self.consensus.leader.view,
            proposal: BlockHeader {
                parent: self.consensus.replica.high_vote.proposal.hash(),
                number: self.consensus.replica.high_vote.proposal.number.next(),
                payload: payload.hash(),
            },
            proposal_payload: Some(payload),
            justification: rng.gen(),
        };

        mutate_fn(&mut msg);

        self.consensus.inner.secret_key.sign_msg(msg)
    }

    pub(crate) fn new_current_replica_commit(
        &self,
        mutate_fn: impl FnOnce(&mut ReplicaCommit),
    ) -> Signed<ReplicaCommit> {
        let mut msg = ReplicaCommit {
            protocol_version: validator::ProtocolVersion::EARLIEST,
            view: self.consensus.replica.view,
            proposal: self.consensus.replica.high_qc.message.proposal,
        };
        mutate_fn(&mut msg);
        self.consensus.inner.secret_key.sign_msg(msg)
    }

    pub(crate) fn new_rnd_leader_commit(
        &mut self,
        rng: &mut impl Rng,
        mutate_fn: impl FnOnce(&mut LeaderCommit),
    ) -> Signed<LeaderCommit> {
        let mut msg = LeaderCommit {
            protocol_version: validator::ProtocolVersion::EARLIEST,
            justification: rng.gen(),
        };
        mutate_fn(&mut msg);
        self.consensus.inner.secret_key.sign_msg(msg)
    }

    pub(crate) async fn new_procedural_leader_prepare(&mut self, ctx: &ctx::Ctx) -> Signed<LeaderPrepare> {
        let replica_prepare = self.new_current_replica_prepare(|_| {});
        self.process_replica_prepare(ctx, replica_prepare).await.unwrap().unwrap()
    }

    pub(crate) async fn new_procedural_leader_prepare_many(&mut self, ctx: &ctx::Ctx) -> Signed<LeaderPrepare> {
        let replica_prepare = self.new_current_replica_prepare(|_| {}).cast().unwrap().msg;
        self.process_replica_prepare_many(
            ctx,
            vec![replica_prepare; self.consensus_threshold()],
            self.keys(),
        )
        .await.unwrap()
    }

    pub(crate) async fn new_procedural_replica_commit(&mut self, ctx: &ctx::Ctx) -> Signed<ReplicaCommit> {
        let leader_prepare = self.new_procedural_leader_prepare(ctx).await;
        self.process_leader_prepare(ctx,leader_prepare).await.unwrap()
    }

    pub(crate) async fn new_procedural_replica_commit_many(&mut self, ctx: &ctx::Ctx) -> Signed<ReplicaCommit> {
        let leader_prepare = self.new_procedural_leader_prepare_many(ctx).await;
        self.process_leader_prepare(ctx,leader_prepare).await.unwrap()
    }

    pub(crate) async fn new_procedural_leader_commit(&mut self, ctx: &ctx::Ctx) -> Signed<LeaderCommit> {
        let replica_commit = self.new_procedural_replica_commit(ctx).await;
        self.process_replica_commit(ctx, replica_commit).await.unwrap().unwrap()
    }

    pub(crate) async fn new_procedural_leader_commit_many(&mut self, ctx: &ctx::Ctx) -> Signed<LeaderCommit> {
        let replica_commit = self.new_procedural_replica_commit_many(ctx).await;
        self.process_replica_commit_many(
            ctx,
            vec![replica_commit.msg; self.consensus_threshold()],
            self.keys(),
        ).await.unwrap()
    }

    pub(crate) async fn process_leader_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<LeaderPrepare>,
    ) -> Result<Signed<ReplicaCommit>, LeaderPrepareError> {
        self
            .consensus
            .replica
            .process_leader_prepare(&ctx, &self.consensus.inner, msg)
            .await?;
        Ok(self.try_recv().unwrap())
    }

    pub(crate) async fn process_leader_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<LeaderCommit>,
    ) -> Result<Signed<ReplicaPrepare>, LeaderCommitError> {
        self
            .consensus
            .replica
            .process_leader_commit(ctx, &self.consensus.inner, msg)
            .await?;
        Ok(self.try_recv().unwrap())
    }

    #[allow(clippy::result_large_err)]
    pub(crate) async fn process_replica_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<ReplicaPrepare>,
    ) -> Result<Option<Signed<LeaderPrepare>>, ReplicaPrepareError> {
        self.consensus
            .leader
            .process_replica_prepare(ctx, &self.consensus.inner, msg)
            .await?;
        Ok(self.try_recv())
    }

    pub(crate) async fn process_replica_prepare_many(
        &mut self,
        ctx: &ctx::Ctx,
        messages: Vec<ReplicaPrepare>,
        keys: Vec<SecretKey>,
    ) -> Option<Signed<LeaderPrepare>> {
        let mut out = None;
        for (i, (msg, key)) in messages.into_iter().zip(keys).enumerate() {
            let res = self.process_replica_prepare(ctx,key.sign_msg(msg)).await;
            match (i + 1).cmp(&self.consensus_threshold()) {
                Ordering::Equal => { out = Some(res.unwrap().unwrap()); },
                Ordering::Less => assert_matches!(
                    res,
                    Err(ReplicaPrepareError::NumReceivedBelowThreshold {
                        num_messages,
                        threshold,
                    }) => {
                        assert_eq!(num_messages, i+1);
                        assert_eq!(threshold, self.consensus_threshold())
                    }
                ),
                Ordering::Greater => assert_matches!(res, Err(ReplicaPrepareError::Old { .. })),
            }
        }
        out
    }

    pub(crate) async fn process_replica_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<ReplicaCommit>,
    ) -> Result<Option<Signed<LeaderCommit>>, ReplicaCommitError> {
        self.consensus.leader.process_replica_commit(
            &ctx,
            &self.consensus.inner,
            msg,
        )?;
        Ok(self.try_recv())
    }

    pub(crate) async fn process_replica_commit_many(
        &mut self,
        ctx: &ctx::Ctx,
        messages: Vec<ReplicaCommit>,
        keys: Vec<SecretKey>,
    ) -> Option<Signed<LeaderCommit>> {
        let mut out = None;
        for (i, (msg, key)) in messages.into_iter().zip(keys).enumerate() {
            let res = self.process_replica_commit(ctx,key.sign_msg(msg)).await;
            match (i + 1).cmp(&self.consensus_threshold()) {
                Ordering::Equal => { out = Some(res.unwrap().unwrap()); },
                Ordering::Less => assert_matches!(
                    res,
                    Err(ReplicaCommitError::NumReceivedBelowThreshold {
                        num_messages,
                        threshold,
                    }) => {
                        assert_eq!(num_messages, i);
                        assert_eq!(threshold, self.consensus_threshold())
                    }
                ),
                Ordering::Greater => assert_matches!(res, Err(ReplicaCommitError::Old { .. })),
            }
        }
        out
    }

    pub(crate) fn try_recv<V:Variant<validator::Msg>>(&mut self) -> Option<Signed<V>> {
        self.pipe
            .try_recv()
            .map(|message| match message {
                OutputMessage::Network(ConsensusInputMessage {
                    message, ..
                }) => message.cast().unwrap(),
            })
    }

    pub(crate) fn current_replica_view(&self) -> ViewNumber {
        self.consensus.replica.view
    }

    pub(crate) fn current_replica_phase(&self) -> Phase {
        self.consensus.replica.phase
    }

    pub(crate) fn view_leader(&self, view: ViewNumber) -> validator::PublicKey {
        self.consensus.inner.view_leader(view)
    }

    pub(crate) fn new_commit_qc(&self, mutate_fn: impl FnOnce(&mut ReplicaCommit)) -> CommitQC {
        let validator_set =
            validator::ValidatorSet::new(self.keys.iter().map(|k| k.public())).unwrap();

        let msg = self
            .new_current_replica_commit(mutate_fn)
            .cast()
            .unwrap()
            .msg;

        let signed_messages: Vec<_> = self.keys.iter().map(|sk| sk.sign_msg(msg)).collect();

        CommitQC::from(&signed_messages, &validator_set).unwrap()
    }

    pub(crate) fn new_prepare_qc(&self, mutate_fn: impl FnOnce(&mut ReplicaPrepare)) -> PrepareQC {
        let validator_set =
            validator::ValidatorSet::new(self.keys.iter().map(|k| k.public())).unwrap();
        let msg = self.new_current_replica_prepare(mutate_fn).msg;
        let signed_messages: Vec<_> = self
            .keys
            .iter()
            .map(|sk| sk.sign_msg(msg.clone()))
            .collect();
        PrepareQC::from(&signed_messages, &validator_set).unwrap()
    }

    pub(crate) fn new_prepare_qc_many(
        &mut self,
        mutate_fn: impl FnMut(&mut ReplicaPrepare),
    ) -> PrepareQC {
        let validator_set =
            validator::ValidatorSet::new(self.keys.iter().map(|k| k.public())).unwrap();

        let signed_messages: Vec<_> = self
            .keys
            .iter()
            .map(|sk| {
                let msg = self.new_current_replica_prepare(|msg| mutate_fn(msg)).msg;
                sk.sign_msg(msg)
            })
            .collect();

        PrepareQC::from(&signed_messages, &validator_set).unwrap()
    }
}
