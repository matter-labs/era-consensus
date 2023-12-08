use crate::{
    io::{InputMessage, OutputMessage},
    leader::{ReplicaCommitError, ReplicaPrepareError},
    replica::{LeaderCommitError, LeaderPrepareError},
    Consensus,
};
use assert_matches::assert_matches;
use rand::Rng;
use std::cmp::Ordering;
use zksync_concurrency::ctx;
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator::{
    self, BlockHeader, CommitQC, LeaderCommit, LeaderPrepare, Payload, Phase, PrepareQC,
    ReplicaCommit, ReplicaPrepare, SecretKey, Signed, ViewNumber,
};
use zksync_consensus_utils::{enum_util::Variant, pipe::DispatcherPipe};

/// `UTHarness` provides various utilities for unit tests.
/// It is designed to simplify the setup and execution of test cases by encapsulating
/// common testing functionality.
///
/// It should be instantiated once for every test case.
#[cfg(test)]
pub(crate) struct UTHarness {
    pub(crate) consensus: Consensus,
    pub(crate) keys: Vec<SecretKey>,
    pipe: DispatcherPipe<InputMessage, OutputMessage>,
}

impl UTHarness {
    /// Creates a new `UTHarness` with the specified validator set size.
    pub(crate) async fn new(ctx: &ctx::Ctx, num_validators: usize) -> UTHarness {
        let mut rng = ctx.rng();
        let keys: Vec<_> = (0..num_validators).map(|_| rng.gen()).collect();
        let (genesis, val_set) =
            crate::testonly::make_genesis(&keys, Payload(vec![]), validator::BlockNumber(0));
        let (mut consensus, pipe) =
            crate::testonly::make_consensus(ctx, &keys[0], &val_set, &genesis).await;

        consensus.leader.view = ViewNumber(1);
        consensus.replica.view = ViewNumber(1);
        UTHarness {
            consensus,
            pipe,
            keys,
        }
    }

    /// Creates a new `UTHarness` with minimally-significant validator set size.
    pub(crate) async fn new_many(ctx: &ctx::Ctx) -> UTHarness {
        let num_validators = 6;
        assert!(crate::misc::faulty_replicas(num_validators) > 0);
        UTHarness::new(ctx, num_validators).await
    }

    /// Triggers replica timeout, validates the new ReplicaPrepare
    /// then executes the whole new view to make sure that the consensus
    /// recovers after a timeout.
    pub(crate) async fn produce_block_after_timeout(&mut self, ctx: &ctx::Ctx) {
        let want = ReplicaPrepare {
            protocol_version: self.protocol_version(),
            view: self.consensus.replica.view.next(),
            high_qc: self.consensus.replica.high_qc.clone(),
            high_vote: self.consensus.replica.high_vote,
        };
        let replica_prepare = self.process_replica_timeout(ctx).await;
        assert_eq!(want, replica_prepare.msg);

        let leader_commit = self.new_leader_commit(ctx).await;
        self.process_leader_commit(ctx, leader_commit)
            .await
            .unwrap();
    }

    pub(crate) fn consensus_threshold(&self) -> usize {
        crate::misc::consensus_threshold(self.keys.len())
    }

    pub(crate) fn protocol_version(&self) -> validator::ProtocolVersion {
        Consensus::PROTOCOL_VERSION
    }

    pub(crate) fn incompatible_protocol_version(&self) -> validator::ProtocolVersion {
        validator::ProtocolVersion(self.protocol_version().0 + 1)
    }

    pub(crate) fn owner_key(&self) -> &SecretKey {
        &self.consensus.inner.secret_key
    }

    pub(crate) fn set_owner_as_view_leader(&mut self) {
        let mut view = self.consensus.replica.view;
        while self.view_leader(view) != self.owner_key().public() {
            view = view.next();
        }
        self.consensus.replica.view = view;
    }

    pub(crate) fn set_view(&mut self, view: ViewNumber) {
        self.set_replica_view(view);
        self.set_leader_view(view);
    }

    pub(crate) fn set_leader_view(&mut self, view: ViewNumber) {
        self.consensus.leader.view = view
    }

    pub(crate) fn set_replica_view(&mut self, view: ViewNumber) {
        self.consensus.replica.view = view
    }

    pub(crate) fn new_replica_prepare(
        &mut self,
        mutate_fn: impl FnOnce(&mut ReplicaPrepare),
    ) -> Signed<ReplicaPrepare> {
        self.set_owner_as_view_leader();
        let mut msg = ReplicaPrepare {
            protocol_version: self.protocol_version(),
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
            protocol_version: self.protocol_version(),
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
            protocol_version: self.protocol_version(),
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
            protocol_version: self.protocol_version(),
            justification: rng.gen(),
        };
        mutate_fn(&mut msg);
        self.consensus.inner.secret_key.sign_msg(msg)
    }

    pub(crate) async fn new_leader_prepare(&mut self, ctx: &ctx::Ctx) -> Signed<LeaderPrepare> {
        let replica_prepare = self.new_replica_prepare(|_| {}).msg;
        self.process_replica_prepare_all(ctx, replica_prepare).await
    }

    pub(crate) async fn new_replica_commit(&mut self, ctx: &ctx::Ctx) -> Signed<ReplicaCommit> {
        let leader_prepare = self.new_leader_prepare(ctx).await;
        self.process_leader_prepare(ctx, leader_prepare)
            .await
            .unwrap()
    }

    pub(crate) async fn new_leader_commit(&mut self, ctx: &ctx::Ctx) -> Signed<LeaderCommit> {
        let replica_commit = self.new_replica_commit(ctx).await;
        self.process_replica_commit_all(ctx, replica_commit.msg)
            .await
    }

    pub(crate) async fn process_leader_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<LeaderPrepare>,
    ) -> Result<Signed<ReplicaCommit>, LeaderPrepareError> {
        self.consensus
            .replica
            .process_leader_prepare(ctx, &self.consensus.inner, msg)
            .await?;
        Ok(self.try_recv().unwrap())
    }

    pub(crate) async fn process_leader_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<LeaderCommit>,
    ) -> Result<Signed<ReplicaPrepare>, LeaderCommitError> {
        self.consensus
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

    pub(crate) async fn process_replica_prepare_all(
        &mut self,
        ctx: &ctx::Ctx,
        msg: ReplicaPrepare,
    ) -> Signed<LeaderPrepare> {
        for (i, key) in self.keys.iter().enumerate() {
            let res = self
                .consensus
                .leader
                .process_replica_prepare(ctx, &self.consensus.inner, key.sign_msg(msg.clone()))
                .await;
            match (i + 1).cmp(&self.consensus_threshold()) {
                Ordering::Equal => res.unwrap(),
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
        self.try_recv().unwrap()
    }

    pub(crate) async fn process_replica_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<ReplicaCommit>,
    ) -> Result<Option<Signed<LeaderCommit>>, ReplicaCommitError> {
        self.consensus
            .leader
            .process_replica_commit(ctx, &self.consensus.inner, msg)?;
        Ok(self.try_recv())
    }

    async fn process_replica_commit_all(
        &mut self,
        ctx: &ctx::Ctx,
        msg: ReplicaCommit,
    ) -> Signed<LeaderCommit> {
        for (i, key) in self.keys.iter().enumerate() {
            let res = self.consensus.leader.process_replica_commit(
                ctx,
                &self.consensus.inner,
                key.sign_msg(msg),
            );
            match (i + 1).cmp(&self.consensus_threshold()) {
                Ordering::Equal => res.unwrap(),
                Ordering::Less => assert_matches!(
                    res,
                    Err(ReplicaCommitError::NumReceivedBelowThreshold {
                        num_messages,
                        threshold,
                    }) => {
                        assert_eq!(num_messages, i+1);
                        assert_eq!(threshold, self.consensus_threshold())
                    }
                ),
                Ordering::Greater => assert_matches!(res, Err(ReplicaCommitError::Old { .. })),
            }
        }
        self.try_recv().unwrap()
    }

    fn try_recv<V: Variant<validator::Msg>>(&mut self) -> Option<Signed<V>> {
        self.pipe.try_recv().map(|message| match message {
            OutputMessage::Network(ConsensusInputMessage { message, .. }) => {
                message.cast().unwrap()
            }
        })
    }

    pub(crate) async fn process_replica_timeout(
        &mut self,
        ctx: &ctx::Ctx,
    ) -> Signed<ReplicaPrepare> {
        self.consensus
            .replica
            .process_input(ctx, &self.consensus.inner, None)
            .await
            .unwrap();
        self.try_recv().unwrap()
    }

    pub(crate) fn leader_phase(&self) -> Phase {
        self.consensus.leader.phase
    }

    pub(crate) fn view_leader(&self, view: ViewNumber) -> validator::PublicKey {
        self.consensus.inner.view_leader(view)
    }

    pub(crate) fn validator_set(&self) -> validator::ValidatorSet {
        validator::ValidatorSet::new(self.keys.iter().map(|k| k.public())).unwrap()
    }

    pub(crate) fn new_commit_qc(&self, mutate_fn: impl FnOnce(&mut ReplicaCommit)) -> CommitQC {
        let msg = self.new_current_replica_commit(mutate_fn).msg;
        let msgs: Vec<_> = self.keys.iter().map(|k| k.sign_msg(msg)).collect();
        CommitQC::from(&msgs, &self.validator_set()).unwrap()
    }

    pub(crate) fn new_prepare_qc(
        &mut self,
        mutate_fn: impl FnOnce(&mut ReplicaPrepare),
    ) -> PrepareQC {
        let msg = self.new_replica_prepare(mutate_fn).msg;
        let msgs: Vec<_> = self.keys.iter().map(|k| k.sign_msg(msg.clone())).collect();
        PrepareQC::from(&msgs, &self.validator_set()).unwrap()
    }
}
