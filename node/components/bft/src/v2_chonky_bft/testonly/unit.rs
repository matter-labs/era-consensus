use std::sync::Arc;

use assert_matches::assert_matches;
use zksync_concurrency::{
    ctx,
    sync::{self, prunable_mpsc},
    time,
};
use zksync_consensus_engine::{
    testonly::{in_memory, TestEngine},
    EngineManagerRunner,
};
use zksync_consensus_roles::validator;
use zksync_consensus_utils::enum_util::Variant;

use crate::{
    create_input_channel,
    v2_chonky_bft::{self, commit, new_view, proposal, timeout, StateMachine},
    Config, FromNetworkMessage, ToNetworkMessage,
};

pub(crate) const MAX_PAYLOAD_SIZE: usize = 1000;

/// `UnitTestHarness` provides various utilities for unit tests.
/// It is designed to simplify the setup and execution of test cases by encapsulating
/// common testing functionality.
///
/// It should be instantiated once for every test case.
#[cfg(test)]
pub(crate) struct UnitTestHarness {
    pub(crate) replica: StateMachine,
    pub(crate) keys: Vec<validator::SecretKey>,
    pub(crate) outbound_channel: ctx::channel::UnboundedReceiver<ToNetworkMessage>,
    pub(crate) inbound_channel: prunable_mpsc::Sender<FromNetworkMessage>,
    pub(crate) _proposer_channel:
        sync::watch::Receiver<Option<validator::v2::ProposalJustification>>,
}

impl UnitTestHarness {
    /// Creates a new `UnitTestHarness` with the specified validator set size.
    pub(crate) async fn new(
        ctx: &ctx::Ctx,
        num_validators: usize,
    ) -> (UnitTestHarness, EngineManagerRunner) {
        Self::new_with_payload_manager(
            ctx,
            num_validators,
            in_memory::PayloadManager::Random(MAX_PAYLOAD_SIZE),
        )
        .await
    }

    /// Creates a new `UnitTestHarness` with minimally-significant validator set size.
    pub(crate) async fn new_many(ctx: &ctx::Ctx) -> (UnitTestHarness, EngineManagerRunner) {
        let num_validators = 6;
        let (util, runner) = UnitTestHarness::new(ctx, num_validators).await;
        assert!(util.validators().max_faulty_weight() > 0);
        (util, runner)
    }

    pub(crate) async fn new_with_payload_manager(
        ctx: &ctx::Ctx,
        num_validators: usize,
        payload_manager: in_memory::PayloadManager,
    ) -> (UnitTestHarness, EngineManagerRunner) {
        let rng = &mut ctx.rng();
        let setup = validator::testonly::Setup::new(rng, num_validators);
        let engine = TestEngine::new_with_payload_manager(ctx, &setup, payload_manager).await;
        let (output_channel_send, output_channel_recv) = ctx::channel::unbounded();
        let (input_channel_send, input_channel_recv) = create_input_channel();
        let (proposer_sender, proposer_receiver) = sync::watch::channel(None);

        let cfg = Arc::new(
            Config::new(
                setup.validator_keys[0].clone(),
                MAX_PAYLOAD_SIZE,
                time::Duration::milliseconds(2000),
                engine.manager.clone(),
                validator::EpochNumber(0),
            )
            .unwrap(),
        );
        let replica = StateMachine::start(
            ctx,
            cfg.clone(),
            output_channel_send.clone(),
            input_channel_recv,
            proposer_sender,
        )
        .await
        .unwrap();
        let mut this = UnitTestHarness {
            replica,
            keys: setup.validator_keys.clone(),
            outbound_channel: output_channel_recv,
            inbound_channel: input_channel_send,
            _proposer_channel: proposer_receiver,
        };

        let timeout = this.new_replica_timeout(ctx).await;
        this.process_replica_timeout_all(ctx, timeout).await;

        (this, engine.runner)
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

    pub(crate) fn view(&self) -> validator::v2::View {
        validator::v2::View {
            genesis: self.genesis_hash(),
            number: self.replica.view_number,
            epoch: self.epoch(),
        }
    }

    pub(crate) fn epoch(&self) -> validator::EpochNumber {
        self.replica.config.epoch
    }

    pub(crate) fn genesis_hash(&self) -> validator::GenesisHash {
        self.replica.config.genesis_hash()
    }

    pub(crate) fn validators(&self) -> &validator::Schedule {
        &self.replica.config.validators
    }

    pub(crate) fn first_block(&self) -> validator::BlockNumber {
        self.replica.config.first_block
    }

    pub(crate) fn view_leader(&self, view: validator::ViewNumber) -> validator::PublicKey {
        self.validators().view_leader(view)
    }

    pub(crate) async fn new_leader_proposal(
        &self,
        ctx: &ctx::Ctx,
    ) -> validator::v2::LeaderProposal {
        let justification = self.replica.get_justification();
        v2_chonky_bft::proposer::create_proposal(ctx, self.replica.config.clone(), justification)
            .await
            .unwrap()
    }

    pub(crate) async fn new_replica_commit(
        &mut self,
        ctx: &ctx::Ctx,
    ) -> validator::v2::ReplicaCommit {
        let proposal = self.new_leader_proposal(ctx).await;
        self.process_leader_proposal(ctx, self.leader_key().sign_msg(proposal))
            .await
            .unwrap()
            .msg
    }

    pub(crate) async fn new_replica_timeout(
        &mut self,
        ctx: &ctx::Ctx,
    ) -> validator::v2::ReplicaTimeout {
        self.replica.start_timeout(ctx).await.unwrap();

        // We *may* have received a new view message before the timeout message.
        match self.try_recv().unwrap().msg {
            validator::ConsensusMsg::V2(m) => match m {
                validator::v2::ChonkyMsg::ReplicaTimeout(msg) => msg,
                // If we did get a new view first, the second message is certainly a timeout.
                validator::v2::ChonkyMsg::ReplicaNewView(_) => self.try_recv().unwrap().msg,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }

    pub(crate) async fn new_replica_new_view(
        &mut self,
        ctx: &ctx::Ctx,
    ) -> validator::v2::ReplicaNewView {
        validator::v2::ReplicaNewView {
            justification: validator::v2::ProposalJustification::Timeout(
                self.new_timeout_qc(ctx).await,
            ),
        }
    }

    pub(crate) async fn new_commit_qc(
        &mut self,
        ctx: &ctx::Ctx,
        mutate_fn: impl FnOnce(&mut validator::v2::ReplicaCommit),
    ) -> validator::v2::CommitQC {
        let mut msg = self.new_replica_commit(ctx).await;
        mutate_fn(&mut msg);
        let mut qc = validator::v2::CommitQC::new(msg.clone(), self.validators());
        for key in &self.keys {
            qc.add(
                &key.sign_msg(msg.clone()),
                self.genesis_hash(),
                self.epoch(),
                self.validators(),
            )
            .unwrap();
        }
        qc
    }

    pub(crate) async fn new_timeout_qc(&mut self, ctx: &ctx::Ctx) -> validator::v2::TimeoutQC {
        let msg = self.new_replica_timeout(ctx).await;
        let mut qc = validator::v2::TimeoutQC::new(msg.view);
        for key in &self.keys {
            qc.add(
                &key.sign_msg(msg.clone()),
                self.genesis_hash(),
                self.epoch(),
                self.validators(),
            )
            .unwrap();
        }
        qc
    }

    pub(crate) async fn process_leader_proposal(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::Signed<validator::v2::LeaderProposal>,
    ) -> Result<validator::Signed<validator::v2::ReplicaCommit>, proposal::Error> {
        self.replica.on_proposal(ctx, msg).await?;
        Ok(self.try_recv().unwrap())
    }

    pub(crate) async fn process_replica_commit(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::Signed<validator::v2::ReplicaCommit>,
    ) -> Result<Option<validator::Signed<validator::v2::ReplicaNewView>>, commit::Error> {
        self.replica.on_commit(ctx, msg).await?;
        Ok(self.try_recv())
    }

    pub(crate) async fn process_replica_timeout(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::Signed<validator::v2::ReplicaTimeout>,
    ) -> Result<Option<validator::Signed<validator::v2::ReplicaNewView>>, timeout::Error> {
        self.replica.on_timeout(ctx, msg).await?;
        Ok(self.try_recv())
    }

    pub(crate) async fn process_replica_new_view(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::Signed<validator::v2::ReplicaNewView>,
    ) -> Result<Option<validator::Signed<validator::v2::ReplicaNewView>>, new_view::Error> {
        self.replica.on_new_view(ctx, msg).await?;
        Ok(self.try_recv())
    }

    pub(crate) async fn process_replica_commit_all(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::v2::ReplicaCommit,
    ) -> validator::Signed<validator::v2::ReplicaNewView> {
        let mut threshold_reached = false;
        let mut cur_weight = 0;

        for key in self.keys.iter() {
            let res = self.replica.on_commit(ctx, key.sign_msg(msg.clone())).await;
            let val_index = self.validators().index(&key.public()).unwrap();

            cur_weight += self.validators().get(val_index).unwrap().weight;

            if threshold_reached {
                assert_matches!(res, Err(commit::Error::Old { .. }));
            } else {
                res.unwrap();
                if cur_weight >= self.validators().quorum_threshold() {
                    threshold_reached = true;
                }
            }
        }

        self.try_recv().unwrap()
    }

    pub(crate) async fn process_replica_timeout_all(
        &mut self,
        ctx: &ctx::Ctx,
        msg: validator::v2::ReplicaTimeout,
    ) -> validator::Signed<validator::v2::ReplicaNewView> {
        let mut threshold_reached = false;
        let mut cur_weight = 0;

        for key in self.keys.iter() {
            let res = self
                .replica
                .on_timeout(ctx, key.sign_msg(msg.clone()))
                .await;
            let val_index = self.validators().index(&key.public()).unwrap();

            cur_weight += self.validators().get(val_index).unwrap().weight;
            if threshold_reached {
                assert_matches!(res, Err(timeout::Error::Old { .. }));
            } else {
                res.unwrap();
                if cur_weight >= self.validators().quorum_threshold() {
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

        // Now to get the timeout message.
        let replica_timeout = match self.try_recv().unwrap().msg {
            // We *may* have received a new view message before the timeout message.
            validator::ConsensusMsg::V2(m) => match m {
                validator::v2::ChonkyMsg::ReplicaTimeout(msg) => msg,
                // If we did get a new view first, the second message is certainly a timeout.
                validator::v2::ChonkyMsg::ReplicaNewView(_) => self.try_recv().unwrap().msg,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };

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
