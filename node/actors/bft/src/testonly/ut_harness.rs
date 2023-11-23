use crate::{
    io::{InputMessage, OutputMessage},
    leader::{ReplicaCommitError, ReplicaPrepareError},
    replica::{LeaderCommitError, LeaderPrepareError},
    Consensus,
};
use rand::{rngs::StdRng, Rng};
use zksync_concurrency::{ctx, ctx::Ctx, scope};
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator::{
    self, BlockHeader, CommitQC, ConsensusMsg, LeaderCommit, LeaderPrepare, Payload, Phase,
    PrepareQC, ReplicaCommit, ReplicaPrepare, SecretKey, Signed, ViewNumber,
};
use zksync_consensus_utils::pipe::DispatcherPipe;

#[cfg(test)]
pub(crate) struct UTHarness {
    ctx: Ctx,
    rng: StdRng,
    consensus: Consensus,
    pipe: DispatcherPipe<InputMessage, OutputMessage>,
    keys: Vec<SecretKey>,
}

impl UTHarness {
    pub(crate) async fn new() -> UTHarness {
        UTHarness::new_with(1).await
    }

    pub(crate) async fn new_with(num_validators: i32) -> UTHarness {
        let ctx = ctx::test_root(&ctx::RealClock);
        let mut rng = ctx.rng();
        let keys: Vec<_> = (0..num_validators).map(|_| rng.gen()).collect();
        let (genesis, val_set) = crate::testonly::make_genesis(&keys, validator::Payload(vec![]));
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

    pub(crate) fn own_key(&self) -> &SecretKey {
        &self.consensus.inner.secret_key
    }

    pub(crate) fn key_at(&self, index: usize) -> &SecretKey {
        &self.keys[index]
    }

    pub(crate) fn rng(&mut self) -> &mut StdRng {
        &mut self.rng
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

    pub(crate) fn set_replica_phase(&mut self, phase: Phase) {
        self.consensus.replica.phase = phase
    }

    pub(crate) fn new_replica_prepare(
        &mut self,
        mutate_fn: impl FnOnce(&mut ReplicaPrepare),
    ) -> Signed<ConsensusMsg> {
        let mut msg = ReplicaPrepare {
            protocol_version: validator::CURRENT_VERSION,
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
            protocol_version: validator::CURRENT_VERSION,
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

    pub(crate) fn new_rnd_replica_commit(&self) -> Signed<ConsensusMsg> {
        let msg = ReplicaCommit {
            protocol_version: validator::CURRENT_VERSION,
            view: self.consensus.replica.view,
            proposal: BlockHeader {
                parent: self.consensus.replica.high_vote.proposal.hash(),
                number: self.consensus.replica.high_vote.proposal.number.next(),
                payload: self.consensus.replica.high_vote.proposal.payload,
            },
        };

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
            protocol_version: validator::CURRENT_VERSION,
            justification: self.rng().gen(),
        };

        mutate_fn(&mut msg);

        self.consensus
            .inner
            .secret_key
            .sign_msg(ConsensusMsg::LeaderCommit(msg))
    }

    pub(crate) async fn new_procedural_leader_prepare(&mut self) -> Signed<ConsensusMsg> {
        let replica_prepare = self.new_replica_prepare(|_| {});
        self.dispatch_replica_prepare(replica_prepare.clone())
            .unwrap();
        self.recv_signed().await.unwrap()
    }

    pub(crate) async fn new_procedural_replica_commit(&mut self) -> Signed<ConsensusMsg> {
        let replica_prepare = self.new_replica_prepare(|_| {});
        self.dispatch_replica_prepare(replica_prepare.clone())
            .unwrap();
        let leader_prepare = self.recv_signed().await.unwrap();
        self.dispatch_leader_prepare(leader_prepare).await.unwrap();
        self.recv_signed().await.unwrap()
    }

    pub(crate) async fn new_procedural_leader_commit(&mut self) -> Signed<ConsensusMsg> {
        let replica_prepare = self.new_replica_prepare(|_| {});
        self.dispatch_replica_prepare(replica_prepare.clone())
            .unwrap();
        let leader_prepare = self.recv_signed().await.unwrap();
        self.dispatch_leader_prepare(leader_prepare).await.unwrap();
        let replica_commit = self.recv_signed().await.unwrap();
        self.dispatch_replica_commit(replica_commit).unwrap();
        self.recv_signed().await.unwrap()
    }

    pub(crate) fn dispatch_replica_prepare(
        &mut self,
        msg: Signed<ConsensusMsg>,
    ) -> Result<(), ReplicaPrepareError> {
        self.consensus.leader.process_replica_prepare(
            &self.ctx,
            &self.consensus.inner,
            msg.cast().unwrap(),
        )
    }

    pub(crate) fn dispatch_replica_commit(
        &mut self,
        msg: Signed<ConsensusMsg>,
    ) -> Result<(), ReplicaCommitError> {
        self.consensus.leader.process_replica_commit(
            &self.ctx,
            &self.consensus.inner,
            msg.cast().unwrap(),
        )
    }

    pub(crate) async fn dispatch_leader_prepare(
        &mut self,
        msg: Signed<ConsensusMsg>,
    ) -> Result<(), LeaderPrepareError> {
        scope::run!(&self.ctx, |ctx, s| {
            s.spawn_blocking(|| {
                Ok(self.consensus.replica.process_leader_prepare(
                    ctx,
                    &self.consensus.inner,
                    msg.cast().unwrap(),
                ))
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
            s.spawn_blocking(|| {
                Ok(self.consensus.replica.process_leader_commit(
                    ctx,
                    &self.consensus.inner,
                    msg.cast().unwrap(),
                ))
            })
            .join(ctx)
        })
        .await
        .unwrap()
    }

    pub(crate) async fn recv_signed(&mut self) -> Option<Signed<ConsensusMsg>> {
        let msg = self.pipe.recv(&self.ctx).await.unwrap();
        match msg {
            OutputMessage::Network(ConsensusInputMessage {
                message: signed, ..
            }) => Some(signed),
            _ => None,
        }
    }

    pub(crate) fn to_leader_prepare(&self, msg: OutputMessage) -> Option<LeaderPrepare> {
        match msg {
            OutputMessage::Network(ConsensusInputMessage {
                message:
                    Signed {
                        msg: ConsensusMsg::LeaderPrepare(leader_prepare),
                        ..
                    },
                ..
            }) => Some(leader_prepare),
            _ => None,
        }
    }

    pub(crate) fn current_view(&self) -> ViewNumber {
        self.consensus.replica.view
    }

    pub(crate) fn current_phase(&self) -> Phase {
        self.consensus.replica.phase
    }

    pub(crate) fn view_leader(&self, view: ViewNumber) -> validator::PublicKey {
        self.consensus.inner.view_leader(view)
    }

    pub(crate) fn new_commit_qc(&self, view: ViewNumber) -> CommitQC {
        let validator_set =
            validator::ValidatorSet::new(self.keys.iter().map(|k| k.public())).unwrap();
        let signed_messages: Vec<_> = self
            .keys
            .iter()
            .map(|sk| {
                sk.sign_msg(validator::ReplicaCommit {
                    protocol_version: validator::CURRENT_VERSION,
                    view,
                    proposal: self.consensus.replica.high_qc.message.proposal,
                })
            })
            .collect();

        CommitQC::from(&signed_messages, &validator_set).unwrap()
    }

    pub(crate) fn new_prepare_qc(&self, msg: &ReplicaPrepare) -> PrepareQC {
        let validator_set =
            validator::ValidatorSet::new(self.keys.iter().map(|k| k.public())).unwrap();
        let signed_messages: Vec<_> = self
            .keys
            .iter()
            .map(|sk| sk.sign_msg(msg.clone()))
            .collect();

        PrepareQC::from(&signed_messages, &validator_set).unwrap()
    }
}
