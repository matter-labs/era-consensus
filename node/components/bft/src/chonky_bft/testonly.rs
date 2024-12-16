use crate::{
    chonky_bft::{self, commit, new_view, proposal, timeout, StateMachine},
    create_input_channel,
    testonly::RandomPayload,
    Config, FromNetworkMessage, PayloadManager, ToNetworkMessage,
};
use assert_matches::assert_matches;
use std::sync::Arc;
use zksync_concurrency::{
    ctx,
    sync::{self, prunable_mpsc},
    time,
};
use zksync_consensus_roles::validator;
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
    pub(crate) replica: StateMachine,
    pub(crate) keys: Vec<validator::SecretKey>,
    pub(crate) outbound_channel: ctx::channel::UnboundedReceiver<ToNetworkMessage>,
    pub(crate) inbound_channel: prunable_mpsc::Sender<FromNetworkMessage>,
    pub(crate) _proposer_channel: sync::watch::Receiver<Option<validator::ProposalJustification>>,
}

impl UTHarness {
    /// Creates a new `UTHarness` with the specified validator set size.
    pub(crate) async fn new(
        ctx: &ctx::Ctx,
        num_validators: usize,
    ) -> (UTHarness, BlockStoreRunner) {
        Self::new_with_payload_manager(
            ctx,
            num_validators,
            Box::new(RandomPayload(MAX_PAYLOAD_SIZE)),
        )
        .await
    }

    /// Creates a new `UTHarness` with minimally-significant validator set size.
    pub(crate) async fn new_many(ctx: &ctx::Ctx) -> (UTHarness, BlockStoreRunner) {
        let num_validators = 6;
        let (util, runner) = UTHarness::new(ctx, num_validators).await;
        assert!(util.genesis().validators.max_faulty_weight() > 0);
        (util, runner)
    }

    pub(crate) async fn new_with_payload_manager(
        ctx: &ctx::Ctx,
        num_validators: usize,
        payload_manager: Box<dyn PayloadManager>,
    ) -> (UTHarness, BlockStoreRunner) {
        let rng = &mut ctx.rng();
        let setup = validator::testonly::Setup::new(rng, num_validators);
        let store = TestMemoryStorage::new(ctx, &setup).await;
        let (output_channel_send, output_channel_recv) = ctx::channel::unbounded();
        let (input_channel_send, input_channel_recv) = create_input_channel();
        let (proposer_sender, proposer_receiver) = sync::watch::channel(None);

        let cfg = Arc::new(Config {
            secret_key: setup.validator_keys[0].clone(),
            block_store: store.blocks.clone(),
            replica_store: Box::new(in_memory::ReplicaStore::default()),
            payload_manager,
            max_payload_size: MAX_PAYLOAD_SIZE,
            view_timeout: time::Duration::milliseconds(2000),
        });
        let replica = StateMachine::start(
            ctx,
            cfg.clone(),
            output_channel_send.clone(),
            input_channel_recv,
            proposer_sender,
        )
        .await
        .unwrap();
        let mut this = UTHarness {
            replica,
            keys: setup.validator_keys.clone(),
            outbound_channel: output_channel_recv,
            inbound_channel: input_channel_send,
            _proposer_channel: proposer_receiver,
        };

        let timeout = this.new_replica_timeout(ctx).await;
        this.process_replica_timeout_all(ctx, timeout).await;

        (this, store.runner)
    }

    pub(crate) fn owner_key(&self) -> &validator::SecretKey {
        &self.replica.config.secret_key
    }

    pub(crate) fn leader_key(&self) -> validator::SecretKey {
        let leader = self.view_leader(self.replica.view_number);
        self.keys
            .iter()
            .find(|key| key.public() == leader)
            .unwrap()
            .clone()
    }

    pub(crate) fn view(&self) -> validator::View {
        validator::View {
            genesis: self.genesis().hash(),
            number: self.replica.view_number,
        }
    }

    pub(crate) fn view_leader(&self, view: validator::ViewNumber) -> validator::PublicKey {
        self.genesis().view_leader(view)
    }

    pub(crate) fn genesis(&self) -> &validator::Genesis {
        self.replica.config.genesis()
    }

    pub(crate) async fn new_leader_proposal(&self, ctx: &ctx::Ctx) -> validator::LeaderProposal {
        let justification = self.replica.get_justification();
        chonky_bft::proposer::create_proposal(ctx, self.replica.config.clone(), justification)
            .await
            .unwrap()
    }

    pub(crate) async fn new_replica_commit(&mut self, ctx: &ctx::Ctx) -> validator::ReplicaCommit {
        let proposal = self.new_leader_proposal(ctx).await;
        self.process_leader_proposal(ctx, self.leader_key().sign_msg(proposal))
            .await
            .unwrap()
            .msg
    }

    pub(crate) async fn new_replica_timeout(
        &mut self,
        ctx: &ctx::Ctx,
    ) -> validator::ReplicaTimeout {
        self.replica.start_timeout(ctx).await.unwrap();
        self.try_recv().unwrap().msg
    }

    pub(crate) async fn new_replica_new_view(
        &mut self,
        ctx: &ctx::Ctx,
    ) -> validator::ReplicaNewView {
        validator::ReplicaNewView {
            justification: validator::ProposalJustification::Timeout(
                self.new_timeout_qc(ctx).await,
            ),
        }
    }

    pub(crate) async fn new_commit_qc(
        &mut self,
        ctx: &ctx::Ctx,
        mutate_fn: impl FnOnce(&mut validator::ReplicaCommit),
    ) -> validator::CommitQC {
        let mut msg = self.new_replica_commit(ctx).await;
        mutate_fn(&mut msg);
        let mut qc = validator::CommitQC::new(msg.clone(), self.genesis());
        for key in &self.keys {
            qc.add(&key.sign_msg(msg.clone()), self.genesis()).unwrap();
        }
        qc
    }

    pub(crate) async fn new_timeout_qc(&mut self, ctx: &ctx::Ctx) -> validator::TimeoutQC {
        let msg = self.new_replica_timeout(ctx).await;
        let mut qc = validator::TimeoutQC::new(msg.view);
        for key in &self.keys {
            qc.add(&key.sign_msg(msg.clone()), self.genesis()).unwrap();
        }
        qc
    }

    pub(crate) async fn process_leader_proposal(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::Signed<validator::LeaderProposal>,
    ) -> Result<validator::Signed<validator::ReplicaCommit>, proposal::Error> {
        self.replica.on_proposal(ctx, msg).await?;
        Ok(self.try_recv().unwrap())
    }

    pub(crate) async fn process_replica_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::Signed<validator::ReplicaCommit>,
    ) -> Result<Option<validator::Signed<validator::ReplicaNewView>>, commit::Error> {
        self.replica.on_commit(ctx, msg).await?;
        Ok(self.try_recv())
    }

    pub(crate) async fn process_replica_timeout(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::Signed<validator::ReplicaTimeout>,
    ) -> Result<Option<validator::Signed<validator::ReplicaNewView>>, timeout::Error> {
        self.replica.on_timeout(ctx, msg).await?;
        Ok(self.try_recv())
    }

    pub(crate) async fn process_replica_new_view(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::Signed<validator::ReplicaNewView>,
    ) -> Result<Option<validator::Signed<validator::ReplicaNewView>>, new_view::Error> {
        self.replica.on_new_view(ctx, msg).await?;
        Ok(self.try_recv())
    }

    pub(crate) async fn process_replica_commit_all(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::ReplicaCommit,
    ) -> validator::Signed<validator::ReplicaNewView> {
        let mut threshold_reached = false;
        let mut cur_weight = 0;

        for key in self.keys.iter() {
            let res = self.replica.on_commit(ctx, key.sign_msg(msg.clone())).await;
            let val_index = self.genesis().validators.index(&key.public()).unwrap();

            cur_weight += self.genesis().validators.get(val_index).unwrap().weight;

            if threshold_reached {
                assert_matches!(res, Err(commit::Error::Old { .. }));
            } else {
                res.unwrap();
                if cur_weight >= self.genesis().validators.quorum_threshold() {
                    threshold_reached = true;
                }
            }
        }

        self.try_recv().unwrap()
    }

    pub(crate) async fn process_replica_timeout_all(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::ReplicaTimeout,
    ) -> validator::Signed<validator::ReplicaNewView> {
        let mut threshold_reached = false;
        let mut cur_weight = 0;

        for key in self.keys.iter() {
            let res = self
                .replica
                .on_timeout(ctx, key.sign_msg(msg.clone()))
                .await;
            let val_index = self.genesis().validators.index(&key.public()).unwrap();

            cur_weight += self.genesis().validators.get(val_index).unwrap().weight;

            if threshold_reached {
                assert_matches!(res, Err(timeout::Error::Old { .. }));
            } else {
                res.unwrap();
                if cur_weight >= self.genesis().validators.quorum_threshold() {
                    threshold_reached = true;
                }
            }
        }

        self.try_recv().unwrap()
    }

    /// Produces a block, by executing the full view.
    pub(crate) async fn produce_block(&mut self, ctx: &ctx::Ctx) {
        let replica_commit = self.new_replica_commit(ctx).await;
        self.process_replica_commit_all(ctx, replica_commit).await;
    }

    /// Triggers replica timeout, processes the new validator::ReplicaTimeout
    /// to start a new view, then executes the whole new view to make sure
    /// that the consensus recovers after a timeout.
    pub(crate) async fn produce_block_after_timeout(&mut self, ctx: &ctx::Ctx) {
        let cur_view = self.replica.view_number;

        self.replica.start_timeout(ctx).await.unwrap();
        let replica_timeout = self.try_recv().unwrap().msg;
        self.process_replica_timeout_all(ctx, replica_timeout).await;

        assert_eq!(self.replica.view_number, cur_view.next());

        self.produce_block(ctx).await;
    }

    pub(crate) fn send(&self, msg: validator::Signed<validator::ConsensusMsg>) {
        self.inbound_channel.send(FromNetworkMessage {
            msg,
            ack: zksync_concurrency::oneshot::channel().0,
        });
    }

    fn try_recv<V: Variant<validator::Msg>>(&mut self) -> Option<validator::Signed<V>> {
        self.outbound_channel
            .try_recv()
            .map(|message| message.message.cast().unwrap())
    }
}
