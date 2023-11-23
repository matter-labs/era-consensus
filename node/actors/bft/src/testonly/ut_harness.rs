use crate::{
    io::{InputMessage, OutputMessage},
    leader::ReplicaPrepareError,
    replica::LeaderPrepareError,
    Consensus,
};
use rand::{rngs::StdRng, Rng};
use zksync_concurrency::{ctx, ctx::Ctx, scope};
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator::{
    self, BlockHeader, CommitQC, ConsensusMsg, LeaderPrepare, Payload, Phase, PrepareQC,
    ReplicaPrepare, SecretKey, Signed, ViewNumber,
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
    pub async fn new() -> UTHarness {
        UTHarness::new_with(1).await
    }

    pub async fn new_with(num_validators: i32) -> UTHarness {
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

    pub fn own_key(&self) -> &SecretKey {
        &self.consensus.inner.secret_key
    }

    pub fn key_at(&self, index: usize) -> &SecretKey {
        &self.keys[index]
    }

    pub fn rng(&mut self) -> &mut StdRng {
        &mut self.rng
    }

    pub fn set_view(&mut self, view: ViewNumber) {
        self.set_replica_view(view);
        self.set_leader_view(view);
    }

    pub fn set_leader_view(&mut self, view: ViewNumber) {
        self.consensus.leader.view = view
    }

    pub fn set_leader_phase(&mut self, phase: Phase) {
        self.consensus.leader.phase = phase
    }

    pub fn set_replica_view(&mut self, view: ViewNumber) {
        self.consensus.replica.view = view
    }

    pub fn set_replica_phase(&mut self, phase: Phase) {
        self.consensus.replica.phase = phase
    }

    pub fn new_replica_prepare(
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

    pub async fn new_procedural_leader_prepare(&mut self) -> Signed<ConsensusMsg> {
        let replica_prepare = self.new_replica_prepare(|_| {});
        self.dispatch_replica_prepare(replica_prepare.clone())
            .unwrap();
        self.recv_signed().await.unwrap()
    }

    pub(crate) fn new_leader_prepare(
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

    pub fn dispatch_replica_prepare(
        &mut self,
        msg: Signed<ConsensusMsg>,
    ) -> Result<(), ReplicaPrepareError> {
        self.consensus.leader.process_replica_prepare(
            &self.ctx,
            &self.consensus.inner,
            msg.cast().unwrap(),
        )
    }

    pub async fn dispatch_leader_prepare(
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

    pub async fn recv_signed(&mut self) -> Option<Signed<ConsensusMsg>> {
        let msg = self.pipe.recv(&self.ctx).await.unwrap();
        match msg {
            OutputMessage::Network(ConsensusInputMessage {
                message: signed, ..
            }) => Some(signed),
            _ => None,
        }
    }

    pub fn to_leader_prepare(&self, msg: OutputMessage) -> Option<LeaderPrepare> {
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

    pub fn current_view(&self) -> ViewNumber {
        self.consensus.replica.view
    }

    pub fn current_phase(&self) -> Phase {
        self.consensus.replica.phase
    }

    pub fn view_leader(&self, view: ViewNumber) -> validator::PublicKey {
        self.consensus.inner.view_leader(view)
    }

    pub fn new_commit_qc(&self, view: ViewNumber) -> CommitQC {
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

    pub fn new_prepare_qc(&self, msg: &ReplicaPrepare) -> PrepareQC {
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

// pub(crate) fn make_replica_prepare(
//     consensus: &Consensus,
//     mutate_callback: Option<impl FnOnce(&mut ReplicaPrepare)>,
// ) -> Signed<ConsensusMsg> {
//     let mut msg = ReplicaPrepare {
//         protocol_version: validator::CURRENT_VERSION,
//         view: consensus.replica.view,
//         high_vote: consensus.replica.high_vote,
//         high_qc: consensus.replica.high_qc.clone(),
//     };
//
//     if let Some(mutate_callback) = mutate_callback {
//         mutate_callback(&mut msg);
//     }
//     consensus
//         .inner
//         .secret_key
//         .sign_msg(validator::ConsensusMsg::ReplicaPrepare(msg))
// }
//
// pub(crate) fn make_replica_commit(
//     consensus: &Consensus,
//     proposal: &BlockHeader,
//     mutate_callback: Option<impl FnOnce(&mut ReplicaCommit)>,
// ) -> Signed<ConsensusMsg> {
//     let mut msg = ReplicaCommit {
//         protocol_version: validator::CURRENT_VERSION,
//         view: consensus.replica.view,
//         proposal: proposal.clone(),
//     };
//     if let Some(mutate_callback) = mutate_callback {
//         mutate_callback(&mut msg);
//     }
//     consensus
//         .inner
//         .secret_key
//         .sign_msg(validator::ConsensusMsg::ReplicaCommit(msg))
// }
//
// pub(crate) fn make_leader_prepare(
//     consensus: &Consensus,
//     proposal_payload: Option<Payload>,
//     justification: PrepareQC,
//     mutate_callback: Option<impl FnOnce(&mut LeaderPrepare)>,
// ) -> Signed<ConsensusMsg> {
//     let mut msg = LeaderPrepare {
//         protocol_version: validator::CURRENT_VERSION,
//         view: consensus.leader.view,
//         proposal: BlockHeader {
//             parent: consensus.replica.high_vote.proposal.hash(),
//             number: consensus.replica.high_vote.proposal.number.next(),
//             payload: proposal_payload.as_ref().unwrap().hash(),
//         },
//         proposal_payload,
//         justification,
//     };
//
//     if let Some(mutate_callback) = mutate_callback {
//         mutate_callback(&mut msg);
//     }
//     consensus
//         .inner
//         .secret_key
//         .sign_msg(ConsensusMsg::LeaderPrepare(msg))
// }
//
// pub(crate) fn make_leader_commit(
//     consensus: &Consensus,
//     proposal_payload: Option<Payload>,
//     justification: PrepareQC,
//     mutate_callback: Option<impl FnOnce(&mut LeaderPrepare)>,
// ) -> Signed<ConsensusMsg> {
//     todo!();
//     let mut msg = LeaderPrepare {
//         protocol_version: validator::CURRENT_VERSION,
//         view: consensus.leader.view,
//         proposal: BlockHeader {
//             parent: consensus.replica.high_vote.proposal.hash(),
//             number: consensus.replica.high_vote.proposal.number.next(),
//             payload: proposal_payload.as_ref().unwrap().hash(),
//         },
//         proposal_payload,
//         justification,
//     };
//
//     if let Some(mutate_callback) = mutate_callback {
//         mutate_callback(&mut msg);
//     }
//     consensus
//         .inner
//         .secret_key
//         .sign_msg(ConsensusMsg::LeaderPrepare(msg))
// }
//
// pub(crate) fn make_leader_prepare_from_replica_prepare(
//     consensus: &Consensus,
//     rng: &mut impl Rng,
//     replica_prepare: Signed<ConsensusMsg>,
//     mutate_callback: Option<impl FnOnce(&mut LeaderPrepare)>,
// ) -> Signed<ConsensusMsg> {
//     let prepare_qc = PrepareQC::from(
//         &[replica_prepare.cast().unwrap()],
//         &consensus.inner.validator_set,
//     )
//     .unwrap();
//     let proposal_payload: Option<Payload> = Some(rng.gen());
//     make_leader_prepare(&consensus, proposal_payload, prepare_qc, mutate_callback)
// }
