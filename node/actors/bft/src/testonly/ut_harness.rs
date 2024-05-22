use crate::{
    io::OutputMessage,
    leader,
    leader::{replica_commit, replica_prepare},
    replica,
    replica::{leader_commit, leader_prepare},
    testonly, Config, PayloadManager,
};
use assert_matches::assert_matches;
use std::sync::Arc;
use zksync_concurrency::{ctx, sync::prunable_mpsc};
use zksync_consensus_network as network;
use zksync_consensus_roles::validator::{
    self, CommitQC, LeaderCommit, LeaderPrepare, Phase, PrepareQC, ReplicaCommit, ReplicaPrepare,
    SecretKey, Signed, ViewNumber,
};
use zksync_consensus_storage::{
    testonly::{in_memory, TestMemoryStorage},
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
    pub(crate) leader_send: prunable_mpsc::Sender<network::io::ConsensusReq>,
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
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        let (send, recv) = ctx::channel::unbounded();

        let cfg = Arc::new(Config {
            secret_key: setup.validator_keys[0].clone(),
            block_store: store.blocks.0.clone(),
            replica_store: Box::new(in_memory::ReplicaStore::default()),
            payload_manager,
            max_payload_size: MAX_PAYLOAD_SIZE,
        });
        let (leader, leader_send) = leader::StateMachine::new(ctx, cfg.clone(), send.clone());
        let (replica, _) = replica::StateMachine::start(ctx, cfg.clone(), send.clone())
            .await
            .unwrap();
        let mut this = UTHarness {
            leader,
            replica,
            pipe: recv,
            keys: setup.validator_keys.clone(),
            leader_send,
        };
        let _: Signed<ReplicaPrepare> = this.try_recv().unwrap();
        (this, store.blocks.1)
    }

    /// Creates a new `UTHarness` with minimally-significant validator set size.
    pub(crate) async fn new_many(ctx: &ctx::Ctx) -> (UTHarness, BlockStoreRunner) {
        let num_validators = 6;
        let (util, runner) = UTHarness::new(ctx, num_validators).await;
        assert!(util.genesis().validators.max_faulty_weight() > 0);
        (util, runner)
    }

    /// Triggers replica timeout, validates the new ReplicaPrepare
    /// then executes the whole new view to make sure that the consensus
    /// recovers after a timeout.
    pub(crate) async fn produce_block_after_timeout(&mut self, ctx: &ctx::Ctx) {
        let want = ReplicaPrepare {
            view: validator::View {
                genesis: self.genesis().hash(),
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

    pub(crate) fn replica_view(&self) -> validator::View {
        validator::View {
            genesis: self.genesis().hash(),
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
        let mut leader_prepare = None;
        let msgs: Vec<_> = self.keys.iter().map(|k| k.sign_msg(msg.clone())).collect();
        let mut first_match = true;
        for (i, msg) in msgs.into_iter().enumerate() {
            let res = self.process_replica_prepare(ctx, msg).await;
            match (
                (i + 1) as u64 * self.genesis().validators.iter().next().unwrap().weight
                    < self.genesis().validators.threshold(),
                first_match,
            ) {
                (true, _) => assert!(res.unwrap().is_none()),
                (false, true) => {
                    first_match = false;
                    leader_prepare = res.unwrap()
                }
                (false, false) => assert_matches!(res, Err(replica_prepare::Error::Old { .. })),
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
        let mut first_match = true;
        for (i, key) in self.keys.iter().enumerate() {
            let res = self
                .leader
                .process_replica_commit(ctx, key.sign_msg(msg.clone()));
            match (
                (i + 1) as u64 * self.genesis().validators.iter().next().unwrap().weight
                    < self.genesis().validators.threshold(),
                first_match,
            ) {
                (true, _) => res.unwrap(),
                (false, true) => {
                    first_match = false;
                    res.unwrap()
                }
                (false, false) => assert_matches!(res, Err(replica_commit::Error::Old { .. })),
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
        self.genesis().view_leader(view)
    }

    pub(crate) fn genesis(&self) -> &validator::Genesis {
        self.replica.config.genesis()
    }

    pub(crate) fn new_commit_qc(&self, mutate_fn: impl FnOnce(&mut ReplicaCommit)) -> CommitQC {
        let mut msg = self.new_current_replica_commit();
        mutate_fn(&mut msg);
        let mut qc = CommitQC::new(msg, self.genesis());
        for key in &self.keys {
            qc.add(&key.sign_msg(qc.message.clone()), self.genesis())
                .unwrap();
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
            qc.add(&key.sign_msg(msg.clone()), self.genesis()).unwrap();
        }
        qc
    }

    pub(crate) fn leader_send(&self, msg: Signed<validator::ConsensusMsg>) {
        self.leader_send.send(network::io::ConsensusReq {
            msg,
            ack: zksync_concurrency::oneshot::channel().0,
        });
    }
}
