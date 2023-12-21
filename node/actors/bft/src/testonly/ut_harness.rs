use crate::{
    testonly,
    io::OutputMessage,
    leader,
    leader::{ReplicaCommitError, ReplicaPrepareError},
    replica,
    replica::{LeaderCommitError, LeaderPrepareError},
    Config,
};
use assert_matches::assert_matches;
use rand::Rng;
use std::{cmp::Ordering, sync::Arc};
use zksync_concurrency::ctx;
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator::{
    self, CommitQC, LeaderCommit, LeaderPrepare, Payload, Phase, PrepareQC, ReplicaCommit,
    ReplicaPrepare, SecretKey, Signed, ViewNumber,
};
use zksync_consensus_storage::{testonly::in_memory, BlockStore};
use zksync_consensus_utils::enum_util::Variant;

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

pub(crate) struct Runner(Arc<BlockStore>);

impl Runner {
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        self.0.run_background_tasks(ctx).await
    }
}

impl UTHarness {
    /// Creates a new `UTHarness` with the specified validator set size.
    pub(crate) async fn new(ctx: &ctx::Ctx, num_validators: usize) -> (UTHarness,Runner) {
        let mut rng = ctx.rng();
        let keys: Vec<_> = (0..num_validators).map(|_| rng.gen()).collect();
        let (genesis, validator_set) =
            crate::testonly::make_genesis(&keys, Payload(vec![]), validator::BlockNumber(0));

        // Initialize the storage.
        let block_store = Box::new(in_memory::BlockStore::new(genesis));
        let block_store = Arc::new(BlockStore::new(ctx,block_store,10).await.unwrap());
        // Create the pipe.
        let (send, recv) = ctx::channel::unbounded();

        let cfg = Arc::new(Config {
            secret_key: keys[0].clone(),
            validator_set,
            block_store: block_store.clone(),
            replica_store: Box::new(in_memory::ReplicaStore::default()),
            payload_manager: Box::new(testonly::RandomPayload),
        });
        let leader = leader::StateMachine::new(ctx, cfg.clone(), send.clone());
        let replica = replica::StateMachine::start(ctx, cfg.clone(), send.clone()).await.unwrap();
        let mut this = UTHarness {
            leader,
            replica,
            pipe: recv,
            keys,
        };
        let _: Signed<ReplicaPrepare> = this.try_recv().unwrap();
        (this,Runner(block_store))
    }

    /// Creates a new `UTHarness` with minimally-significant validator set size.
    pub(crate) async fn new_many(ctx: &ctx::Ctx) -> (UTHarness,Runner) {
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
            view: self.replica.view.next(),
            high_qc: self.replica.high_qc.clone(),
            high_vote: self.replica.high_vote,
        };
        let replica_prepare = self.process_replica_timeout(ctx).await;
        assert_eq!(want, replica_prepare.msg);

        let leader_commit = self.new_leader_commit(ctx).await;
        self.process_leader_commit(ctx, leader_commit)
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

    pub(crate) fn new_replica_prepare(
        &mut self,
        mutate_fn: impl FnOnce(&mut ReplicaPrepare),
    ) -> Signed<ReplicaPrepare> {
        self.set_owner_as_view_leader();
        let mut msg = ReplicaPrepare {
            protocol_version: self.protocol_version(),
            view: self.replica.view,
            high_vote: self.replica.high_vote,
            high_qc: self.replica.high_qc.clone(),
        };
        mutate_fn(&mut msg);
        self.owner_key().sign_msg(msg)
    }

    pub(crate) fn new_current_replica_commit(
        &self,
        mutate_fn: impl FnOnce(&mut ReplicaCommit),
    ) -> Signed<ReplicaCommit> {
        let mut msg = ReplicaCommit {
            protocol_version: self.protocol_version(),
            view: self.replica.view,
            proposal: self.replica.high_qc.message.proposal,
        };
        mutate_fn(&mut msg);
        self.owner_key().sign_msg(msg)
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
        self.replica.process_leader_prepare(ctx, msg).await?;
        Ok(self.try_recv().unwrap())
    }

    pub(crate) async fn process_leader_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<LeaderCommit>,
    ) -> Result<Signed<ReplicaPrepare>, LeaderCommitError> {
        self.replica.process_leader_commit(ctx, msg).await?;
        Ok(self.try_recv().unwrap())
    }

    #[allow(clippy::result_large_err)]
    pub(crate) async fn process_replica_prepare(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<ReplicaPrepare>,
    ) -> Result<Option<Signed<LeaderPrepare>>, ReplicaPrepareError> {
        let prepare_qc = self.leader.prepare_qc.subscribe();
        self.leader.process_replica_prepare(ctx, msg).await?;
        if prepare_qc.has_changed().unwrap() {
            let prepare_qc = prepare_qc.borrow().clone().unwrap(); 
            leader::StateMachine::propose(
                ctx,
                &self.leader.config,
                prepare_qc,
                &self.leader.pipe,
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
        let want_threshold = self.replica.config.threshold();
        let mut leader_prepare = None;
        let msgs: Vec<_> = self.keys.iter().map(|k| k.sign_msg(msg.clone())).collect();
        for (i, msg) in msgs.into_iter().enumerate() {
            let res = self.process_replica_prepare(ctx, msg).await;
            match (i + 1).cmp(&want_threshold) {
                Ordering::Equal => leader_prepare = res.unwrap(),
                Ordering::Less => assert!(res.unwrap().is_none()),
                Ordering::Greater => assert_matches!(res, Err(ReplicaPrepareError::Old { .. })),
            }
        }
        leader_prepare.unwrap()
    }

    pub(crate) async fn process_replica_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: Signed<ReplicaCommit>,
    ) -> Result<Option<Signed<LeaderCommit>>, ReplicaCommitError> {
        self.leader.process_replica_commit(ctx, msg)?;
        Ok(self.try_recv())
    }

    async fn process_replica_commit_all(
        &mut self,
        ctx: &ctx::Ctx,
        msg: ReplicaCommit,
    ) -> Signed<LeaderCommit> {
        for (i, key) in self.keys.iter().enumerate() {
            let res = self.leader.process_replica_commit(ctx, key.sign_msg(msg));
            let want_threshold = self.replica.config.threshold();
            match (i + 1).cmp(&want_threshold) {
                Ordering::Equal => res.unwrap(),
                Ordering::Less => res.unwrap(),
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
        self.replica.start_new_view(ctx).await.unwrap();
        self.try_recv().unwrap()
    }

    pub(crate) fn leader_phase(&self) -> Phase {
        self.leader.phase
    }

    pub(crate) fn view_leader(&self, view: ViewNumber) -> validator::PublicKey {
        self.replica.config.view_leader(view)
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
