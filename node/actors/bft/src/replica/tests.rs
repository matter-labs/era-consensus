use crate::testonly;
use rand::Rng;
use zksync_concurrency::{ctx, scope, testonly::abort_on_panic, time};
use zksync_consensus_network::io::{ConsensusInputMessage, Target};
use zksync_consensus_roles::validator::{self, ViewNumber};

#[tokio::test]
async fn start_new_view_not_leader() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::ManualClock::new());
    let rng = &mut ctx.rng();

    let keys: Vec<_> = (0..4).map(|_| rng.gen()).collect();
    let (genesis, val_set) = testonly::make_genesis(&keys, validator::Payload(vec![]));
    let (mut consensus, mut pipe) =
        testonly::make_consensus(ctx, &keys[0], &val_set, &genesis).await;
    // TODO: this test assumes a specific implementation of the leader schedule.
    // Make it leader-schedule agnostic (use epoch to select a specific view).
    consensus.replica.view = ViewNumber(1);
    consensus.replica.high_qc = rng.gen();
    consensus.replica.high_qc.message.view = ViewNumber(0);

    scope::run!(ctx, |ctx, s| {
        s.spawn(async {
            consensus
                .replica
                .start_new_view(ctx, &consensus.inner)
                .await
                .unwrap();
            Ok(())
        })
        .join(ctx)
    })
    .await
    .unwrap();

    let test_new_view_msg = ConsensusInputMessage {
        message: consensus
            .inner
            .secret_key
            .sign_msg(validator::ConsensusMsg::ReplicaPrepare(
                validator::ReplicaPrepare {
                    protocol_version: validator::CURRENT_VERSION,
                    view: consensus.replica.view,
                    high_vote: consensus.replica.high_vote,
                    high_qc: consensus.replica.high_qc.clone(),
                },
            )),
        recipient: Target::Validator(consensus.inner.view_leader(consensus.replica.view)),
    };

    assert_eq!(pipe.recv(ctx).await.unwrap(), test_new_view_msg.into());
    assert!(consensus.replica.timeout_deadline < time::Deadline::Infinite);
}
