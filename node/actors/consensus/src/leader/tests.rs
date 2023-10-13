use crate::{leader::error::Error, testonly};
use concurrency::ctx;
use rand::{rngs::StdRng, Rng, SeedableRng};
use roles::validator;

// TODO(bruno): This only tests a particular case, not the whole method.
#[tokio::test]
async fn replica_commit() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut StdRng::seed_from_u64(6516565651);

    let keys: Vec<_> = (0..1).map(|_| rng.gen()).collect();
    let (genesis, val_set) = testonly::make_genesis(&keys, vec![]);
    let (mut consensus, _) = testonly::make_consensus(ctx, &keys[0], &val_set, &genesis);

    let proposal_block_hash = rng.gen();

    consensus.leader.view = validator::ViewNumber(3);
    consensus.leader.phase = validator::Phase::Commit;

    let test_replica_msg =
        consensus
            .inner
            .secret_key
            .sign_msg(validator::ConsensusMsg::ReplicaCommit(
                validator::ReplicaCommit {
                    view: consensus.leader.view,
                    proposal_block_hash,
                    proposal_block_number: validator::BlockNumber(42),
                },
            ));

    assert_eq!(
        consensus.leader.process_replica_commit(
            ctx,
            &consensus.inner,
            test_replica_msg.cast().unwrap()
        ),
        Err(Error::ReplicaCommitMissingProposal)
    );
}
