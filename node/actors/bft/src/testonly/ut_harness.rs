use crate::{
    io::OutputMessage,
    leader,
    leader::{replica_commit, replica_prepare},
    replica,
    replica::{leader_commit, leader_prepare},
    testonly, Config, PayloadManager,
};
use assert_matches::assert_matches;
use std::{cmp::Ordering, sync::Arc};
use zksync_concurrency::ctx;
use zksync_consensus_network as network;
use zksync_consensus_roles::validator::{
    self, CommitQC, LeaderCommit, LeaderPrepare, Phase, PrepareQC, ReplicaCommit, ReplicaPrepare,
    SecretKey, Signed, ViewNumber,
};
use zksync_consensus_storage::{
    testonly::{in_memory, new_store},
    BlockStoreRunner,
};
use zksync_consensus_utils::enum_util::Variant;

pub(crate) const MAX_PAYLOAD_SIZE: usize = 1000;

/// `UTHarness` provides various utilities for unit tests.
/// It is designed to simplify the setup and execution of test cases by encapsulating
/// common testing functionality.
///
/// It should be instantiated once for every test case.
#[cfg(test)]
pub(crate) struct UTHarness {
    pub(crate) leader: leader::StateMachine,
    pub(crate) replica: replica::StateMachine,
    pub(crate) keys: Vec<SecretKey>,
    pipe: ctx::channel::UnboundedReceiver<OutputMessage>,
}

impl UTHarness {
    /// Creates a new `UTHarness` with the specified validator set size.
    pub(crate) async fn new(
        ctx: &ctx::Ctx,
        num_validators: usize,
    ) -> (UTHarness, BlockStoreRunner) {
        Self::new_with_payload(
            ctx,
            num_validators,
            Box::new(testonly::RandomPayload(MAX_PAYLOAD_SIZE)),
        )
        .await
    }

    pub(crate) async fn new_with_payload(
        ctx: &ctx::Ctx,
        num_validators: usize,
        payload_manager: Box<dyn PayloadManager>,
    ) -> (UTHarness, BlockStoreRunner) {
        let rng = &mut ctx.rng();
        let setup = validator::testonly::Setup::new(rng, num_validators);
        let (block_store, runner) = new_store(ctx, &setup.genesis).await;
        let (send, recv) = ctx::channel::unbounded();

        let cfg = Arc::new(Config {
            secret_key: setup.keys[0].clone(),
            block_store: block_store.clone(),
            replica_store: Box::new(in_memory::ReplicaStore::default()),
            payload_manager,
            max_payload_size: MAX_PAYLOAD_SIZE,
        });
        let (leader, _) = leader::StateMachine::new(ctx, cfg.clone(), send.clone());
        let (replica, _) = replica::StateMachine::start(ctx, cfg.clone(), send.clone())
            .await
            .unwrap();
        let mut this = UTHarness {
            leader,
            replica,
            pipe: recv,
            keys: setup.keys.clone(),
        };
        let _: Signed<ReplicaPrepare> = this.try_recv().unwrap();
        (this, runner)
    }

    /// Creates a new `UTHarness` with minimally-significant validator set size.
    pub(crate) async fn new_many(ctx: &ctx::Ctx) -> (UTHarness, BlockStoreRunner) {
        let num_validators = 6;
        assert!(validator::faulty_replicas(num_validators) > 0);
        UTHarness::new(ctx, num_validators).await
    }

    /// Triggers replica timeout, validates the new ReplicaPrepare
    /// then executes the whole new view to make sure that the consensus
    /// recovers after a timeout.
    pub(crate) async fn produce_block_after_timeout(&mut self, ctx: &ctx::Ctx) {
        let want = ReplicaPrepare {
            view: validator::View {
                protocol_version: self.protocol_version(),
                fork: self.genesis().forks.current().number,
                number: self.replica.view.next(),
            },
            high_qc: self.replica.high_qc.clone(),
            high_vote: self.replica.high_vote.clone(),
        };
        let replica_prepare = self.process_replica_timeout(ctx).await;
        assert_eq!(want, replica_prepare.msg);
        self.produce_block(ctx).await;
    }

    /// Produces a block, by executing the full view.
    pub(crate) async fn produce_block(&mut self, ctx: &ctx::Ctx) {
        let msg = self.new_leader_commit(ctx).await;
        self.process_leader_commit(ctx, self.sign(msg))
            .await
            .unwrap();
    }

    pub(crate) fn protocol_version(&self) -> validator::ProtocolVersion {
        crate::PROTOCOL_VERSION
    }

    pub(crate) fn incompatible_protocol_version(&self) -> validator::ProtocolVersion {
        validator::ProtocolVersion(self.protocol_version().0 + 1)
    }

    pub(crate) fn owner_key(&self) -> &SecretKey {
        &self.replica.config.secret_key
    }

    pub(crate) fn sign<V: Variant<validator::Msg>>(&self, msg: V) -> Signed<V> {
        self.replica.config.secret_key.sign_msg(msg)
    }

    pub(crate) fn set_owner_as_view_leader(&mut self) {
        let mut view = self.replica.view;
        while self.view_leader(view) != self.owner_key().public() {
            view = view.next();
        }
        self.replica.view = view;
    }

    pub(crate) fn set_view(&mut self, view: ViewNumber) {
        self.set_replica_view(view);
        self.set_leader_view(view);
    }

    pub(crate) fn set_leader_view(&mut self, view: ViewNumber) {
        self.leader.view = view
    }

    pub(crate) fn set_replica_view(&mut self, view: ViewNumber) {
        self.replica.view = view
    }

    pub(crate) fn replica_view(&self) -> validator::View {
        validator::View {
            protocol_version: self.protocol_version(),
            fork: self.genesis().forks.current().number,
            number: self.replica.view,
        }
    }

    pub(crate) fn new_replica_prepare(&mut self) -> ReplicaPrepare {
        self.set_owner_as_view_leader();
        ReplicaPrepare {
            view: self.replica_view(),
            high_vote: self.replica.high_vote.clone(),
            high_qc: self.replica.high_qc.clone(),
        }
    }

    pub(crate) fn new_current_replica_commit(&self) -> ReplicaCommit {
        ReplicaCommit {
            view: self.replica_view(),
            proposal: self.replica.high_qc.as_ref().unwrap().message.proposal,
        }
    }

    pub(crate) async fn new_leader_prepare(&mut self, ctx: &ctx::Ctx) -> LeaderPrepare {
        let msg = self.new_replica_prepare();
        self.process_replica_prepare_all(ctx, msg).await.msg
    }

    pub(crate) async fn new_replica_commit(&mut self, ctx: &ctx::Ctx) -> ReplicaCommit {
        let msg = self.new_leader_prepare(ctx).await;
        self.process_leader_prepare(ctx, self.sign(msg))
            .await
            .unwrap()
            .msg
    }

    pub(crate) async fn new_leader_commit(&mut self, ctx: &ctx::Ctx) -> LeaderCommit {
        let msg = self.new_replica_commit(ctx).await;
        self.process_replica_commit_all(ctx, msg).await.msg
    }

    pub(crate) async fn process_leader_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<LeaderPrepare>,
    ) -> Result<Signed<ReplicaCommit>, leader_prepare::Error> {
        self.replica.process_leader_prepare(ctx, msg).await?;
        Ok(self.try_recv().unwrap())
    }

    pub(crate) async fn process_leader_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<LeaderCommit>,
    ) -> Result<Signed<ReplicaPrepare>, leader_commit::Error> {
        self.replica.process_leader_commit(ctx, msg).await?;
        Ok(self.try_recv().unwrap())
    }

    #[allow(clippy::result_large_err)]
    pub(crate) async fn process_replica_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<ReplicaPrepare>,
    ) -> Result<Option<Signed<LeaderPrepare>>, replica_prepare::Error> {
        let prepare_qc = self.leader.prepare_qc.subscribe();
        self.leader.process_replica_prepare(ctx, msg).await?;
        if prepare_qc.has_changed().unwrap() {
            let prepare_qc = prepare_qc.borrow().clone().unwrap();
            leader::StateMachine::propose(
                ctx,
                &self.leader.config,
                prepare_qc,
                &self.leader.outbound_pipe,
            )
            .await
            .unwrap();
        }
        Ok(self.try_recv())
    }

    pub(crate) async fn process_replica_prepare_all(
        &mut self,
        ctx: &ctx::Ctx,
        msg: ReplicaPrepare,
    ) -> Signed<LeaderPrepare> {
        let want_threshold = self.genesis().validators.threshold();
        let mut leader_prepare = None;
        let msgs: Vec<_> = self.keys.iter().map(|k| k.sign_msg(msg.clone())).collect();
        for (i, msg) in msgs.into_iter().enumerate() {
            let res = self.process_replica_prepare(ctx, msg).await;
            match (i + 1).cmp(&want_threshold) {
                Ordering::Equal => leader_prepare = res.unwrap(),
                Ordering::Less => assert!(res.unwrap().is_none()),
                Ordering::Greater => assert_matches!(res, Err(replica_prepare::Error::Old { .. })),
            }
        }
        leader_prepare.unwrap()
    }

    pub(crate) async fn process_replica_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<ReplicaCommit>,
    ) -> Result<Option<Signed<LeaderCommit>>, replica_commit::Error> {
        self.leader.process_replica_commit(ctx, msg)?;
        Ok(self.try_recv())
    }

    async fn process_replica_commit_all(
        &mut self,
        ctx: &ctx::Ctx,
        msg: ReplicaCommit,
    ) -> Signed<LeaderCommit> {
        for (i, key) in self.keys.iter().enumerate() {
            let res = self
                .leader
                .process_replica_commit(ctx, key.sign_msg(msg.clone()));
            let want_threshold = self.genesis().validators.threshold();
            match (i + 1).cmp(&want_threshold) {
                Ordering::Equal => res.unwrap(),
                Ordering::Less => res.unwrap(),
                Ordering::Greater => assert_matches!(res, Err(replica_commit::Error::Old { .. })),
            }
        }
        self.try_recv().unwrap()
    }

    fn try_recv<V: Variant<validator::Msg>>(&mut self) -> Option<Signed<V>> {
        self.pipe.try_recv().map(|message| match message {
            OutputMessage::Network(network::io::ConsensusInputMessage { message, .. }) => {
                message.cast().unwrap()
            }
        })
    }

    pub(crate) async fn process_replica_timeout(
        &mut self,
        ctx: &ctx::Ctx,
    ) -> Signed<ReplicaPrepare> {
        self.replica.start_new_view(ctx).await.unwrap();
        self.try_recv().unwrap()
    }

    pub(crate) fn leader_phase(&self) -> Phase {
        self.leader.phase
    }

    pub(crate) fn view_leader(&self, view: ViewNumber) -> validator::PublicKey {
        self.genesis().validators.view_leader(view)
    }

    pub(crate) fn genesis(&self) -> &validator::Genesis {
        self.replica.config.genesis()
    }

    pub(crate) fn new_commit_qc(&self, mutate_fn: impl FnOnce(&mut ReplicaCommit)) -> CommitQC {
        let mut msg = self.new_current_replica_commit();
        mutate_fn(&mut msg);
        let mut qc = CommitQC::new(msg, self.genesis());
        for key in &self.keys {
            qc.add(&key.sign_msg(qc.message.clone()), self.genesis());
        }
        qc
    }

    pub(crate) fn new_prepare_qc(
        &mut self,
        mutate_fn: impl FnOnce(&mut ReplicaPrepare),
    ) -> PrepareQC {
        let mut msg = self.new_replica_prepare();
        mutate_fn(&mut msg);
        let mut qc = PrepareQC::new(msg.view.clone());
        for key in &self.keys {
            qc.add(&key.sign_msg(msg.clone()), self.genesis());
        }
        qc
    }
}
