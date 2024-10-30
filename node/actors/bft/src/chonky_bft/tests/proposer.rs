use crate::chonky_bft::{self, commit, testonly::UTHarness};
use anyhow::{anyhow, Context};
use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::{ctx, error::Wrap, scope, sync};
use zksync_consensus_roles::validator;

// TODO
/// Sanity check of the happy path.
#[tokio::test]
async fn proposer_sanity() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        let cfg = util.replica.config.clone();
        let outbound_pipe = util.replica.outbound_pipe.clone();
        //let proposer_pipe = util.proposer_pipe.clone();
        let (proposer_sender, proposer_receiver) = sync::watch::channel(None);

        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(async {
            let res =
                chonky_bft::proposer::run_proposer(ctx, cfg, outbound_pipe, proposer_receiver)
                    .await;

            match res {
                Ok(()) => Ok(()),
                Err(ctx::Error::Internal(err)) => Err(err),
                Err(ctx::Error::Canceled(_)) => unreachable!(),
            }
        });

        //util.produce_block(ctx).await;

        Ok(())
    })
    .await
    .unwrap();
}
