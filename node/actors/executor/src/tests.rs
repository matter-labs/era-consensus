//! High-level tests for `Executor`.

use super::*;
use crate::testonly::FullValidatorConfig;
use concurrency::sync;
use roles::validator::{BlockNumber, Payload};
use storage::{BlockStore, InMemoryStorage, StorageError};

async fn run_executor(ctx: &ctx::Ctx, executor: Executor<InMemoryStorage>) -> anyhow::Result<()> {
    executor.run(ctx).await.or_else(|err| {
        if err.root_cause().is::<ctx::Canceled>() {
            Ok(()) // Test has successfully finished
        } else {
            Err(err)
        }
    })
}

async fn store_final_blocks(
    ctx: &ctx::Ctx,
    mut blocks_receiver: channel::UnboundedReceiver<FinalBlock>,
    storage: Arc<InMemoryStorage>,
) -> anyhow::Result<()> {
    while let Ok(block) = blocks_receiver.recv(ctx).await {
        tracing::trace!(number = %block.header.number, "Finalized new block");
        if let Err(err) = storage.put_block(ctx, &block).await {
            if matches!(err, StorageError::Canceled(_)) {
                break;
            } else {
                return Err(err.into());
            }
        }
    }
    Ok(())
}

impl FullValidatorConfig {
    fn into_executor(
        self,
        storage: Arc<InMemoryStorage>,
    ) -> (
        Executor<InMemoryStorage>,
        channel::UnboundedReceiver<FinalBlock>,
    ) {
        let (blocks_sender, blocks_receiver) = channel::unbounded();
        let mut executor = Executor::new(self.node_config, self.node_key, storage.clone()).unwrap();
        executor
            .set_validator(
                self.consensus_config,
                self.validator_key,
                storage,
                blocks_sender,
            )
            .unwrap();
        (executor, blocks_receiver)
    }
}

#[tokio::test]
async fn executing_single_validator() {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let validator = FullValidatorConfig::for_single_validator(rng, Payload(vec![]));
    let genesis_block = &validator.node_config.genesis_block;
    let storage = InMemoryStorage::new(genesis_block.clone());
    let storage = Arc::new(storage);
    let (executor, mut blocks_receiver) = validator.into_executor(storage);

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(run_executor(ctx, executor));

        let mut expected_block_number = BlockNumber(1);
        while expected_block_number < BlockNumber(5) {
            let final_block = blocks_receiver.recv(ctx).await?;
            tracing::trace!(number = %final_block.header.number, "Finalized new block");
            assert_eq!(final_block.header.number, expected_block_number);
            expected_block_number = expected_block_number.next();
        }
        anyhow::Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn executing_validator_and_external_node() {
    concurrency::testonly::abort_on_panic();
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

    let (validator, blocks_receiver) = validator.into_executor(validator_storage.clone());
    let external_node = Executor::new(
        external_node.node_config,
        external_node.node_key,
        external_node_storage.clone(),
    )
    .unwrap();

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(run_executor(ctx, validator));
        s.spawn_bg(run_executor(ctx, external_node));
        s.spawn_bg(store_final_blocks(ctx, blocks_receiver, validator_storage));

        for _ in 0..5 {
            let number = *sync::changed(ctx, &mut en_subscriber).await?;
            tracing::trace!(%number, "External node received block");
        }
        anyhow::Ok(())
    })
    .await
    .unwrap();
}
