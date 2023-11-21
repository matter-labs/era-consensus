use assert_matches::assert_matches;
use rand::Rng;

use zksync_consensus_crypto::bn254::Error::SignatureVerificationFailure;
use zksync_consensus_roles::validator::{
    ConsensusMsg, LeaderPrepare, Phase, ReplicaPrepare, ViewNumber,
};

use crate::{
    leader::ReplicaPrepareError, replica::LeaderPrepareError, tests::unit_tests::util::Util,
};

/// ## Tests coverage
///
/// - [x] replica_prepare_sanity
/// - [ ] replica_prepare_sanity_yield_leader_prepare
/// - [x] replica_prepare_old_view
/// - [x] replica_prepare_during_commit
/// - [ ] replica_prepare_not_leader_in_view
/// - [ ] replica_prepare_already_exists
/// - [ ] replica_prepare_invalid_sig
/// - [ ] replica_prepare_invalid_commit_qc_different_views
/// - [ ] replica_prepare_invalid_commit_qc_signers_list_empty
/// - [ ] replica_prepare_invalid_commit_qc_signers_list_invalid_size
/// - [ ] replica_prepare_invalid_commit_qc_multi_msg_per_signer
/// - [ ] replica_prepare_invalid_commit_qc_insufficient_signers
/// - [ ] replica_prepare_invalid_commit_qc_invalid_aggregate_sig
/// - [ ] replica_prepare_high_qc_of_future_view
/// - [x] replica_prepare_num_received_below_threshold
/// -
/// - [x] leader_prepare_sanity
/// - [ ] leader_prepare_sanity_yield_replica_commit
/// - [x] leader_prepare_invalid_leader
/// - [x] leader_prepare_old_view
/// - [x] leader_prepare_invalid_sig
/// - [x] leader_prepare_invalid_prepare_qc_different_views
/// - [ ] leader_prepare_invalid_prepare_qc_signers_list_empty
/// - [ ] leader_prepare_invalid_prepare_qc_signers_list_invalid_size
/// - [ ] leader_prepare_invalid_prepare_qc_multi_msg_per_signer
/// - [ ] leader_prepare_invalid_prepare_qc_insufficient_signers
/// - [ ] leader_prepare_invalid_prepare_qc_invalid_aggregate_sig
/// - [ ] leader_prepare_high_qc_of_future_view
/// - [ ] leader_prepare_proposal_oversized_payload
/// - [ ] leader_prepare_proposal_mismatched_payload
/// - [ ] leader_prepare_proposal_when_previous_not_finalized
/// - [ ] leader_prepare_proposal_invalid_parent_hash
/// - [ ] leader_prepare_proposal_non_sequential_number
/// - [ ] leader_prepare_reproposal_without_quorum
/// - [ ] leader_prepare_reproposal_when_finalized
/// - [ ] leader_prepare_reproposal_invalid_block
/// - [ ] leader_prepare_yield_replica_commit
/// -
/// - [ ] replica_commit_sanity
/// - [ ] replica_commit_sanity_yield_leader_commit
/// - [ ] replica_commit_old
/// - [ ] replica_commit_not_leader_in_view
/// - [ ] replica_commit_already_exists
/// - [ ] replica_commit_invalid_sig
/// - [ ] replica_commit_unexpected_proposal
/// - [ ] replica_commit_num_received_below_threshold
/// - [ ] replica_commit_make_commit_qc_failure_distinct_messages
/// -
/// - [ ] leader_commit_sanity
/// - [ ] leader_commit_sanity_yield_replica_prepare
/// - [ ] leader_commit_invalid_leader
/// - [ ] leader_commit_old
/// - [ ] leader_commit_invalid_sig
/// - [ ] leader_commit_invalid_commit_qc_signers_list_empty
/// - [ ] leader_commit_invalid_commit_qc_signers_list_invalid_size
/// - [ ] leader_commit_invalid_commit_qc_insufficient_signers
/// - [ ] leader_commit_invalid_commit_qc_invalid_aggregate_sig
///
#[tokio::test]
async fn replica_prepare_sanity() {
    let mut util = Util::new().await;

    let replica_prepare = util.new_replica_prepare(|_| {});
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(res, Ok(()));
}

#[tokio::test]
async fn replica_prepare_sanity_yield_leader_prepare() {
    let mut util = Util::new().await;

    let replica_prepare = util.new_replica_prepare(|_|{});
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(res, Ok(()));
    let _ = util.recv_leader_prepare().await.unwrap();
}

#[tokio::test]
async fn replica_prepare_old_view() {
    let mut util = Util::new().await;

    util.set_replica_view(ViewNumber(1));
    util.set_leader_view(ViewNumber(2));
    util.set_leader_phase(Phase::Prepare);

    let replica_prepare = util.new_replica_prepare(|_|{});
    let res = util.dispatch_replica_prepare(replica_prepare);

    assert_matches!(
        res,
        Err(ReplicaPrepareError::Old {
            current_view: ViewNumber(2),
            current_phase: Phase::Prepare,
        })
    );
}

#[tokio::test]
async fn replica_prepare_during_commit() {
    let mut util = Util::new().await;

    util.set_leader_phase(Phase::Commit);

    let replica_prepare = util.new_replica_prepare(|_|{});
    let res = util.dispatch_replica_prepare(replica_prepare);

    assert_matches!(
        res,
        Err(ReplicaPrepareError::Old {
            current_view: ViewNumber(1),
            current_phase: Phase::Commit,
        })
    );
}

#[tokio::test]
async fn replica_prepare_already_exists() {
    let mut util = Util::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);

    assert_eq!(util.view_leader(view), util.own_key().public());
    let replica_prepare = util.new_replica_prepare(|_|{});

    let res = util.dispatch_replica_prepare(replica_prepare.clone());
    assert_matches!(
        res,
        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
            num_messages: 1,
            threshold: 2
        })
    );

    let res = util.dispatch_replica_prepare(replica_prepare.clone());
    assert_matches!(
    res,
    Err(ReplicaPrepareError::Exists {existing_message: msg }) => {
            assert_eq!(msg, replica_prepare.cast().unwrap().msg);
        }
    );
}

#[tokio::test]
async fn replica_prepare_num_received_below_threshold() {
    let mut util = Util::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);
    assert_eq!(util.view_leader(view), util.own_key().public());

    let replica_prepare = util.new_replica_prepare(|_|{});
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(
        res,
        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
            num_messages: 1,
            threshold: 2
        })
    );
}

#[tokio::test]
async fn leader_prepare_sanity() {
    let mut util = Util::new().await;

    let replica_prepare = util.new_replica_prepare(|_|{});
    let res = util.dispatch_replica_prepare(replica_prepare);
    assert_matches!(res, Ok(()));
    let leader_prepare = util.recv_signed().await.unwrap();
    let res = util.dispatch_leader_prepare(leader_prepare).await;

    assert_matches!(res, Ok(()));
}

#[tokio::test]
async fn leader_prepare_invalid_leader() {
    let mut util = Util::new_with(2).await;

    let view = ViewNumber(2);
    util.set_replica_view(view);
    util.set_leader_view(view);

    assert_eq!(util.view_leader(view), util.key_at(0).public());

    let replica_prepare_one = util.new_replica_prepare(|_|{});
    let res = util.dispatch_replica_prepare(replica_prepare_one.clone());
    assert_matches!(
        res,
        Err(ReplicaPrepareError::NumReceivedBelowThreshold {
            num_messages: 1,
            threshold: 2,
        })
    );

    let replica_prepare_two = util.key_at(1).sign_msg(replica_prepare_one.msg);
    let res = util.dispatch_replica_prepare(replica_prepare_two);
    assert_matches!(res, Ok(()));

    let mut leader_prepare = util.recv_leader_prepare().await.unwrap();
    leader_prepare.view = leader_prepare.view.next();
    assert_ne!(
        util.view_leader(leader_prepare.view),
        util.key_at(0).public()
    );
    let leader_prepare = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));
    let res = util.dispatch_leader_prepare(leader_prepare).await;

    assert_matches!(
        res,
        Err(LeaderPrepareError::InvalidLeader { correct_leader, received_leader }) => {
            assert_eq!(correct_leader, util.key_at(1).public());
            assert_eq!(received_leader, util.key_at(0).public());
        }
    );
}

#[tokio::test]
async fn leader_prepare_invalid_sig() {
    let mut util = Util::new().await;

    let mut leader_prepare = util.new_leader_prepare(|_|{});
    leader_prepare.sig = util.rng().gen();
    let res = util.dispatch_leader_prepare(leader_prepare).await;

    assert_matches!(
        res,
        Err(LeaderPrepareError::InvalidSignature(
            SignatureVerificationFailure
        ))
    );
}

#[tokio::test]
async fn leader_prepare_invalid_prepare_qc_different_views() {
    let mut util = Util::new().await;

    let replica_prepare = util.new_replica_prepare(|_|{});
    let res = util.dispatch_replica_prepare(replica_prepare.clone());
    assert_matches!(res, Ok(()));

    let mut leader_prepare = util.recv_leader_prepare().await.unwrap();
    leader_prepare.view = leader_prepare.view.next();
    let leader_prepare = util
        .own_key()
        .sign_msg(ConsensusMsg::LeaderPrepare(leader_prepare));
    let res = util.dispatch_leader_prepare(leader_prepare).await;

    assert_matches!(
    res,
    Err(LeaderPrepareError::InvalidPrepareQC(err)) => {
        assert_eq!(err.to_string(), "PrepareQC contains messages for different views!")
        }
    )
}

// #[tokio::test]
// async fn replica_commit_sanity() {
//     let mut util = Util::make().await;
//
//     let replica_prepare = util::make_replica_prepare(&consensus, None::<fn(&mut ReplicaPrepare)>);
//     let res = util::dispatch_replica_prepare(&ctx, &mut consensus, replica_prepare);
//     assert_matches!(res, Ok(()));
//     let leader_prepare = util::make_leader_prepare_from_replica_prepare(&consensus, &mut rng, replica_prepare, Some(|msg: &mut LeaderPrepare| {
//         msg.view = ViewNumber(2);
//     }));
//     let replica_commit = util::make_replica_commit(&consensus, leader_prepare.msg., None::<fn(&mut ReplicaCommit)>);
//
//     let res = scope::run!(&ctx, |ctx, s| {
//         s.spawn_blocking(|| {
//             let res = consensus
//             .leader
//             .process_replica_commit(ctx, &consensus.inner, leader_prepare.cast().unwrap());
//             Ok(res)
//         })
//         .join(ctx)
//     })
//         .await
//         .unwrap();
//
//     assert_matches!(
//         res,
//         Err(LeaderPrepareError::InvalidPrepareQC(anyhow!("PrepareQC contains messages for different views!")))
//     );
// }

mod util {
    use rand::{Rng, rngs::StdRng, SeedableRng};

    use zksync_concurrency::{ctx, ctx::Ctx, scope};
    use zksync_consensus_network::io::ConsensusInputMessage;
    use zksync_consensus_roles::{
        validator,
        validator::{
            BlockHeader, ConsensusMsg, LeaderPrepare, Payload, Phase, ReplicaPrepare, SecretKey,
            Signed, ViewNumber,
        },
    };
    use zksync_consensus_utils::pipe::DispatcherPipe;

    use crate::{
        Consensus,
        io::{InputMessage, OutputMessage},
        leader::ReplicaPrepareError,
        replica::LeaderPrepareError,
    };

    pub(crate) struct Util {
        ctx: Ctx,
        consensus: Consensus,
        rng: StdRng,
        pipe: DispatcherPipe<InputMessage, OutputMessage>,
        keys: Vec<SecretKey>,
    }

    impl Util {
        pub async fn new() -> Util {
            Util::new_with(1).await
        }

        pub async fn new_with(num_validators: i32) -> Util {
            let ctx = ctx::test_root(&ctx::RealClock);
            let mut rng = StdRng::seed_from_u64(6516565651);
            let keys: Vec<_> = (0..num_validators).map(|_| rng.gen()).collect();
            let (genesis, val_set) =
                crate::testonly::make_genesis(&keys, validator::Payload(vec![]));
            let (mut consensus, pipe) =
                crate::testonly::make_consensus(&ctx, &keys[0], &val_set, &genesis).await;

            consensus.leader.view = ViewNumber(1);
            consensus.replica.view = ViewNumber(1);

            Util {
                ctx,
                consensus,
                rng,
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

        pub(crate) fn new_leader_prepare(
            &mut self,
            mutate_fn: impl FnOnce(&mut LeaderPrepare),
        ) -> Signed<ConsensusMsg> {
            let payload: Payload = self.rng.gen();
            let mut msg = LeaderPrepare {
                protocol_version: validator::CURRENT_VERSION,
                view: self.consensus.leader.view,
                proposal: BlockHeader {
                    parent: self.consensus.replica.high_vote.proposal.hash(),
                    number: self.consensus.replica.high_vote.proposal.number.next(),
                    payload: payload.hash(),
                },
                proposal_payload: Some(payload),
                justification: self.rng.gen(),
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
            if let OutputMessage::Network(ConsensusInputMessage {
                                              message: signed, ..
                                          }) = msg
            {
                return Some(signed);
            }
            None
        }

        pub async fn recv_leader_prepare(&mut self) -> Option<LeaderPrepare> {
            let msg = self.pipe.recv(&self.ctx).await.unwrap();
            if let OutputMessage::Network(ConsensusInputMessage {
                                              message:
                                              Signed {
                                                  msg: ConsensusMsg::LeaderPrepare(leader_prepare),
                                                  ..
                                              },
                                              ..
                                          }) = msg
            {
                return Some(leader_prepare);
            }
            None
        }

        pub fn view_leader(&self, view: ViewNumber) -> validator::PublicKey {
            self.consensus.inner.view_leader(view)
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
}
