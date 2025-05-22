use rand::Rng as _;
use zksync_concurrency::{ctx, scope, testonly::abort_on_panic};
use zksync_consensus_roles::{
    validator,
    validator::testonly::{Setup, SetupSpec},
};

use crate::testonly::TestEngine;

mod v1;
mod v2;

// Test checking that store doesn't accept pre-genesis blocks with invalid justification.
#[tokio::test]
async fn test_invalid_justification() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let mut spec = SetupSpec::new(rng, 1);
    spec.first_block = spec.first_pregenesis_block + 2;
    let setup = Setup::from_spec(rng, spec);

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));

        // Insert a correct block first.
        engine
            .manager
            .queue_block(ctx, setup.blocks[0].clone())
            .await
            .unwrap();

        // Insert an incorrect second block.
        let validator::Block::PreGenesis(mut b) = setup.blocks[1].clone() else {
            panic!()
        };
        b.justification = rng.gen();
        engine.manager.queue_block(ctx, b.into()).await.unwrap_err();

        Ok(())
    })
    .await
    .unwrap();
}
