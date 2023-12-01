use crate::{
    io::{InputMessage, OutputMessage},
    leader::{ReplicaCommitError, ReplicaPrepareError},
    replica::{LeaderCommitError, LeaderPrepareError},
    Consensus,
};
use assert_matches::assert_matches;
use rand::{rngs::StdRng, Rng};
use zksync_concurrency::{
    ctx,
    ctx::{Canceled, Ctx},
    scope,
};
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator::{
    self, BlockHeader, CommitQC, ConsensusMsg, LeaderCommit, LeaderPrepare, Payload, Phase,
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
    ctx: Ctx,
    rng: StdRng,
    consensus: Consensus,
    pipe: DispatcherPipe<InputMessage, OutputMessage>,
    keys: Vec<SecretKey>,
}

impl UTHarness {
    /// Creates a new `UTHarness` with one validator._
    pub(crate) async fn new_one() -> UTHarness {
        UTHarness::new_with(1).await
    }
    /// Creates a new `UTHarness` with minimally-significant validator set size.
    pub(crate) async fn new_many() -> UTHarness {
        let num_validators = 6;
        assert_matches!(crate::misc::faulty_replicas(num_validators), res if res > 0);
        let mut util = UTHarness::new_with(num_validators).await;
        util.set_replica_view(util.owner_as_view_leader_next());
        util
    }

    /// Creates a new `UTHarness` with the specified validator set size.
    pub(crate) async fn new_with(num_validators: usize) -> UTHarness {
        let ctx = ctx::test_root(&ctx::RealClock);
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

        UTHarness {
            ctx,
            rng,
            consensus,
            pipe,
            keys,
        }
    }

    pub(crate) fn check_recovery_after_timeout(&mut self) {
        self.set_replica_view(self.owner_as_view_leader_next());

        let base_replica_view = self.replica_view();
        let base_leader_view = self.leader_view();
        assert!(base_leader_view < base_replica_view);

        assert_eq!(self.replica_phase(), Phase::Prepare);

        let replica_prepare = self.new_current_replica_prepare(|_| {}).cast().unwrap().msg;
        self.dispatch_replica_prepare_many(
            vec![replica_prepare; self.consensus_threshold()],
            self.keys(),
        )
        .unwrap();

        assert_eq!(self.replica_view(), base_replica_view);
        assert_eq!(self.leader_view(), base_replica_view);
        assert_eq!(self.replica_phase(), Phase::Prepare);
        assert_eq!(self.leader_phase(), Phase::Commit);
    }

    pub(crate) fn consensus_threshold(&self) -> usize {
        crate::misc::consensus_threshold(self.keys.len())
    }

    pub(crate) fn owner_key(&self) -> &SecretKey {
        &self.consensus.inner.secret_key
    }

    pub(crate) fn owner_as_view_leader_next(&self) -> ViewNumber {
        let mut view = self.replica_view();
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

    pub(crate) fn rng(&mut self) -> &mut StdRng {
        &mut self.rng
    }

    pub(crate) fn new_rng(&self) -> StdRng {
        self.ctx.rng()
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

    pub(crate) fn new_unfinalized_replica_prepare(&self) -> Signed<ConsensusMsg> {
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
    ) -> Signed<ConsensusMsg> {
        let mut msg = ReplicaPrepare {
            protocol_version: validator::ProtocolVersion::EARLIEST,
            view: self.consensus.replica.view,
            high_vote: self.consensus.replica.high_vote,
            high_qc: self.consensus.replica.high_qc.clone(),
        };

        mutate_fn(&mut msg);

        self.consensus
            .inner
            .secret_key
            .sign_msg(ConsensusMsg::ReplicaPrepare(msg))
    }

    pub(crate) fn new_rnd_leader_prepare(
        &mut self,
        mutate_fn: impl FnOnce(&mut LeaderPrepare),
    ) -> Signed<ConsensusMsg> {
        let payload: Payload = self.rng().gen();
        let mut msg = LeaderPrepare {
            protocol_version: validator::ProtocolVersion::EARLIEST,
            view: self.consensus.leader.view,
            proposal: BlockHeader {
                parent: self.consensus.replica.high_vote.proposal.hash(),
                number: self.consensus.replica.high_vote.proposal.number.next(),
                payload: payload.hash(),
            },
            proposal_payload: Some(payload),
            justification: self.rng().gen(),
        };

        mutate_fn(&mut msg);

        self.consensus
            .inner
            .secret_key
            .sign_msg(ConsensusMsg::LeaderPrepare(msg))
    }

    pub(crate) fn new_current_replica_commit(
        &self,
        mutate_fn: impl FnOnce(&mut ReplicaCommit),
    ) -> Signed<ConsensusMsg> {
        let mut msg = ReplicaCommit {
            protocol_version: validator::ProtocolVersion::EARLIEST,
            view: self.consensus.replica.view,
            proposal: self.consensus.replica.high_qc.message.proposal,
        };

        mutate_fn(&mut msg);

        self.consensus
            .inner
            .secret_key
            .sign_msg(ConsensusMsg::ReplicaCommit(msg))
    }

    pub(crate) fn new_rnd_leader_commit(
        &mut self,
        mutate_fn: impl FnOnce(&mut LeaderCommit),
    ) -> Signed<ConsensusMsg> {
        let mut msg = LeaderCommit {
            protocol_version: validator::ProtocolVersion::EARLIEST,
            justification: self.rng().gen(),
        };

        mutate_fn(&mut msg);

        self.consensus
            .inner
            .secret_key
            .sign_msg(ConsensusMsg::LeaderCommit(msg))
    }

    pub(crate) async fn new_procedural_leader_prepare_one(&mut self) -> Signed<ConsensusMsg> {
        let replica_prepare = self.new_current_replica_prepare(|_| {});
        self.dispatch_replica_prepare_one(replica_prepare.clone())
            .unwrap();
        self.recv_signed().await.unwrap()
    }

    pub(crate) async fn new_procedural_leader_prepare_many(&mut self) -> Signed<ConsensusMsg> {
        let replica_prepare = self.new_current_replica_prepare(|_| {}).cast().unwrap().msg;
        self.dispatch_replica_prepare_many(
            vec![replica_prepare; self.consensus_threshold()],
            self.keys(),
        )
        .unwrap();
        self.recv_signed().await.unwrap()
    }

    pub(crate) async fn new_procedural_replica_commit_one(&mut self) -> Signed<ConsensusMsg> {
        let replica_prepare = self.new_current_replica_prepare(|_| {});
        self.dispatch_replica_prepare_one(replica_prepare.clone())
            .unwrap();
        let leader_prepare = self.recv_signed().await.unwrap();
        self.dispatch_leader_prepare(leader_prepare).await.unwrap();
        self.recv_signed().await.unwrap()
    }

    pub(crate) async fn new_procedural_replica_commit_many(&mut self) -> Signed<ConsensusMsg> {
        let leader_prepare = self.new_procedural_leader_prepare_many().await;
        self.dispatch_leader_prepare(leader_prepare).await.unwrap();
        self.recv_signed().await.unwrap()
    }

    pub(crate) async fn new_procedural_leader_commit_one(&mut self) -> Signed<ConsensusMsg> {
        let replica_prepare = self.new_current_replica_prepare(|_| {});
        self.dispatch_replica_prepare_one(replica_prepare.clone())
            .unwrap();
        let leader_prepare = self.recv_signed().await.unwrap();
        self.dispatch_leader_prepare(leader_prepare).await.unwrap();
        let replica_commit = self.recv_signed().await.unwrap();
        self.dispatch_replica_commit_one(replica_commit).unwrap();
        self.recv_signed().await.unwrap()
    }

    pub(crate) async fn new_procedural_leader_commit_many(&mut self) -> Signed<ConsensusMsg> {
        let replica_commit = self
            .new_procedural_replica_commit_many()
            .await
            .cast()
            .unwrap()
            .msg;
        self.dispatch_replica_commit_many(
            vec![replica_commit; self.consensus_threshold()],
            self.keys(),
        )
        .unwrap();
        self.recv_signed().await.unwrap()
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn dispatch_replica_prepare_one(
        &mut self,
        msg: Signed<ConsensusMsg>,
    ) -> Result<(), ReplicaPrepareError> {
        self.consensus.leader.process_replica_prepare(
            &self.ctx,
            &self.consensus.inner,
            msg.cast().unwrap(),
        )
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn dispatch_replica_prepare_many(
        &mut self,
        messages: Vec<ReplicaPrepare>,
        keys: Vec<SecretKey>,
    ) -> Result<(), ReplicaPrepareError> {
        let len = messages.len();
        let consensus_threshold = self.consensus_threshold();
        messages
            .into_iter()
            .zip(keys)
            .map(|(msg, key)| {
                let signed = key.sign_msg(ConsensusMsg::ReplicaPrepare(msg));
                self.dispatch_replica_prepare_one(signed)
            })
            .fold((0, None), |(i, _), res| {
                let i = i + 1;
                if i < len {
                    assert_matches!(
                        res,
                        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
                            num_messages,
                            threshold,
                        }) => {
                            assert_eq!(num_messages, i);
                            assert_eq!(threshold, consensus_threshold)
                        }
                    );
                }
                (i, Some(res))
            })
            .1
            .unwrap()
    }

    pub(crate) fn dispatch_replica_commit_one(
        &mut self,
        msg: Signed<ConsensusMsg>,
    ) -> Result<(), ReplicaCommitError> {
        self.consensus.leader.process_replica_commit(
            &self.ctx,
            &self.consensus.inner,
            msg.cast().unwrap(),
        )
    }

    pub(crate) fn dispatch_replica_commit_many(
        &mut self,
        messages: Vec<ReplicaCommit>,
        keys: Vec<SecretKey>,
    ) -> Result<(), ReplicaCommitError> {
        let len = messages.len();
        let consensus_threshold = self.consensus_threshold();
        messages
            .into_iter()
            .zip(keys)
            .map(|(msg, key)| {
                let signed = key.sign_msg(ConsensusMsg::ReplicaCommit(msg));
                self.dispatch_replica_commit_one(signed)
            })
            .fold((0, None), |(i, _), res| {
                let i = i + 1;
                if i < len {
                    assert_matches!(
                        res,
                        Err(ReplicaCommitError::NumReceivedBelowThreshold {
                            num_messages,
                            threshold,
                        }) => {
                            assert_eq!(num_messages, i);
                            assert_eq!(threshold, consensus_threshold)
                        }
                    );
                }
                (i, Some(res))
            })
            .1
            .unwrap()
    }

    pub(crate) async fn dispatch_leader_prepare(
        &mut self,
        msg: Signed<ConsensusMsg>,
    ) -> Result<(), LeaderPrepareError> {
        scope::run!(&self.ctx, |ctx, s| {
            s.spawn(async {
                let res = self
                    .consensus
                    .replica
                    .process_leader_prepare(ctx, &self.consensus.inner, msg.cast().unwrap())
                    .await;
                Ok(res)
            })
            .join(ctx)
        })
        .await
        .unwrap()
    }

    pub(crate) async fn sim_timeout(&mut self) {
        scope::run!(&self.ctx, |ctx, s| {
            s.spawn(async {
                let _ = self
                    .consensus
                    .replica
                    .process_input(ctx, &self.consensus.inner, None)
                    .await;
                Ok(())
            })
            .join(ctx)
        })
        .await
        .unwrap()
    }

    pub(crate) async fn dispatch_leader_commit(
        &mut self,
        msg: Signed<ConsensusMsg>,
    ) -> Result<(), LeaderCommitError> {
        scope::run!(&self.ctx, |ctx, s| {
            s.spawn(async {
                let res = self
                    .consensus
                    .replica
                    .process_leader_commit(ctx, &self.consensus.inner, msg.cast().unwrap())
                    .await;
                Ok(res)
            })
            .join(ctx)
        })
        .await
        .unwrap()
    }

    pub(crate) async fn recv_signed(&mut self) -> Result<Signed<ConsensusMsg>, Canceled> {
        self.pipe
            .recv(&self.ctx)
            .await
            .map(|output_message| match output_message {
                OutputMessage::Network(ConsensusInputMessage {
                    message: signed, ..
                }) => signed,
            })
    }

    pub(crate) fn replica_view(&self) -> ViewNumber {
        self.consensus.replica.view
    }

    pub(crate) fn leader_view(&self) -> ViewNumber {
        self.consensus.leader.view
    }

    pub(crate) fn replica_phase(&self) -> Phase {
        self.consensus.replica.phase
    }

    pub(crate) fn leader_phase(&self) -> Phase {
        self.consensus.leader.phase
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

        let msg: ReplicaPrepare = self
            .new_current_replica_prepare(mutate_fn)
            .cast()
            .unwrap()
            .msg;

        let signed_messages: Vec<_> = self
            .keys
            .iter()
            .map(|sk| sk.sign_msg(msg.clone()))
            .collect();

        PrepareQC::from(&signed_messages, &validator_set).unwrap()
    }

    pub(crate) fn new_prepare_qc_many(
        &mut self,
        mutate_fn: &dyn Fn(&mut ReplicaPrepare),
    ) -> PrepareQC {
        let validator_set =
            validator::ValidatorSet::new(self.keys.iter().map(|k| k.public())).unwrap();

        let signed_messages: Vec<_> = self
            .keys
            .iter()
            .map(|sk| {
                let msg: ReplicaPrepare = self
                    .new_current_replica_prepare(|msg| mutate_fn(msg))
                    .cast()
                    .unwrap()
                    .msg;
                sk.sign_msg(msg.clone())
            })
            .collect();

        PrepareQC::from(&signed_messages, &validator_set).unwrap()
    }
}
