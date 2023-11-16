//! High-level tests for `Executor`.
use super::*;
use crate::testonly::FullValidatorConfig;
use zksync_concurrency::{sync, testonly::abort_on_panic};
use zksync_consensus_roles::validator::{BlockNumber, Payload};
use zksync_consensus_storage::{BlockStore, InMemoryStorage};

impl FullValidatorConfig {
    fn into_executor(self, storage: Arc<InMemoryStorage>) -> Executor<InMemoryStorage> {
        let mut executor = Executor::new(self.node_config, self.node_key, storage.clone()).unwrap();
        executor
            .set_validator(self.consensus_config, self.validator_key, storage)
            .unwrap();
        executor
    }
}

#[tokio::test]
async fn executing_single_validator() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let validator = FullValidatorConfig::for_single_validator(rng, Payload(vec![]));
    let genesis_block = &validator.node_config.genesis_block;
    let storage = InMemoryStorage::new(genesis_block.clone());
    let storage = Arc::new(storage);
    let executor = validator.into_executor(storage.clone());

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(executor.run(ctx));
        let want = BlockNumber(5);
        sync::wait_for(ctx, &mut storage.subscribe_to_block_writes(), |n| {
            n >= &want
        })
        .await?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn executing_validator_and_external_node() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let mut validator = FullValidatorConfig::for_single_validator(rng, Payload(vec![]));
    let external_node = validator.connect_external_node(rng);

    let genesis_block = &validator.node_config.genesis_block;
    let validator_storage = InMemoryStorage::new(genesis_block.clone());
    let validator_storage = Arc::new(validator_storage);
    let external_node_storage = InMemoryStorage::new(genesis_block.clone());
    let external_node_storage = Arc::new(external_node_storage);
    let mut en_subscriber = external_node_storage.subscribe_to_block_writes();

    let validator = validator.into_executor(validator_storage.clone());
    let external_node = Executor::new(
        external_node.node_config,
        external_node.node_key,
        external_node_storage.clone(),
    )
    .unwrap();

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(validator.run(ctx));
        s.spawn_bg(external_node.run(ctx));
        for _ in 0..5 {
            let number = *sync::changed(ctx, &mut en_subscriber).await?;
            tracing::trace!(%number, "External node received block");
        }
        anyhow::Ok(())
    })
    .await
    .unwrap();
}
