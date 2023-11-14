//! High-level tests for `Executor`.

use super::*;
use crate::testonly::FullValidatorConfig;
use std::iter;
use test_casing::test_casing;
use zksync_concurrency::{sync, testonly::abort_on_panic, time};
use zksync_consensus_roles::validator::{BlockNumber, Payload};
use zksync_consensus_storage::{BlockStore, InMemoryStorage, StorageError};

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
    fn gen_blocks(&self, count: usize) -> Vec<FinalBlock> {
        let genesis_block = self.node_config.genesis_block.clone();
        let validators = &self.node_config.validators;
        let blocks = iter::successors(Some(genesis_block), |parent| {
            let payload = Payload(vec![]);
            let header = validator::BlockHeader {
                parent: parent.header.hash(),
                number: parent.header.number.next(),
                payload: payload.hash(),
            };
            let commit = self.validator_key.sign_msg(validator::ReplicaCommit {
                protocol_version: validator::CURRENT_VERSION,
                view: validator::ViewNumber(header.number.0),
                proposal: header,
            });
            let justification = validator::CommitQC::from(&[commit], validators).unwrap();
            Some(FinalBlock::new(header, payload, justification))
        });
        blocks.skip(1).take(count).collect()
    }

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
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let validator = FullValidatorConfig::for_single_validator(rng, Payload(vec![]));
    let genesis_block = &validator.node_config.genesis_block;
    let storage = InMemoryStorage::new(genesis_block.clone());
    let storage = Arc::new(storage);
    let (executor, mut blocks_receiver) = validator.into_executor(storage);

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(executor.run(ctx));

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

    let (validator, blocks_receiver) = validator.into_executor(validator_storage.clone());
    let external_node = Executor::new(
        external_node.node_config,
        external_node.node_key,
        external_node_storage.clone(),
    )
    .unwrap();

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(validator.run(ctx));
        s.spawn_bg(external_node.run(ctx));
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

#[test_casing(2, [false, true])]
#[tokio::test]
async fn syncing_external_node_from_snapshot(delay_block_storage: bool) {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let mut validator = FullValidatorConfig::for_single_validator(rng, Payload(vec![]));
    let external_node = validator.connect_external_node(rng);

    let genesis_block = &validator.node_config.genesis_block;
    let blocks = validator.gen_blocks(10);
    let validator_storage = InMemoryStorage::new(genesis_block.clone());
    let validator_storage = Arc::new(validator_storage);
    if !delay_block_storage {
        // Instead of running consensus on the validator, add the generated blocks manually.
        for block in &blocks {
            validator_storage.put_block(ctx, block).await.unwrap();
        }
    }
    let validator = Executor::new(
        validator.node_config,
        validator.node_key,
        validator_storage.clone(),
    )
    .unwrap();

    // Start an external node from a snapshot.
    let external_node_storage = InMemoryStorage::new(blocks[3].clone());
    let external_node_storage = Arc::new(external_node_storage);
    let mut en_subscriber = external_node_storage.subscribe_to_block_writes();

    let external_node = Executor::new(
        external_node.node_config,
        external_node.node_key,
        external_node_storage.clone(),
    )
    .unwrap();

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(validator.run(ctx));
        s.spawn_bg(external_node.run(ctx));

        if delay_block_storage {
            // Emulate the validator gradually adding new blocks to the storage.
            s.spawn_bg(async {
                for block in &blocks {
                    ctx.sleep(time::Duration::milliseconds(500)).await?;
                    validator_storage.put_block(ctx, block).await?;
                }
                Ok(())
            });
        }

        loop {
            let last_contiguous_en_block = external_node_storage
                .last_contiguous_block_number(ctx)
                .await?;
            tracing::trace!(%last_contiguous_en_block, "External node has last contiguous block");
            if last_contiguous_en_block == BlockNumber(10) {
                break; // The EN has received all blocks!
            }
            // Wait until the EN storage is updated.
            let number = *sync::changed(ctx, &mut en_subscriber).await?;
            tracing::trace!(%number, "External node received block");
        }

        // Check that the node didn't receive any blocks with number lesser than the initial snapshot block.
        for lesser_block_number in 0..3 {
            let block = external_node_storage
                .block(ctx, BlockNumber(lesser_block_number))
                .await?;
            assert!(block.is_none());
        }
        anyhow::Ok(())
    })
    .await
    .unwrap();
}
