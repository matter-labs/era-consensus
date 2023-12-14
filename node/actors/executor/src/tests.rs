//! High-level tests for `Executor`.

use super::*;
use crate::testonly::FullValidatorConfig;
use rand::{thread_rng, Rng};
use std::iter;
use test_casing::test_casing;
use zksync_concurrency::{sync, testonly::abort_on_panic, time};
use zksync_consensus_bft::{testonly::RandomPayloadSource, PROTOCOL_VERSION};
use zksync_consensus_roles::validator::{BlockNumber, FinalBlock, Payload};
use zksync_consensus_storage::{BlockStore, InMemoryStorage};

impl FullValidatorConfig {
    fn gen_blocks(&self, rng: &mut impl Rng, count: usize) -> Vec<FinalBlock> {
        let genesis_block = self.genesis_block.clone();
        let validators = &self.node_config.validators;
        let blocks = iter::successors(Some(genesis_block), |parent| {
            let payload: Payload = rng.gen();
            let header = validator::BlockHeader {
                parent: parent.header.hash(),
                number: parent.header.number.next(),
                payload: payload.hash(),
            };
            let commit = self.validator_key.sign_msg(validator::ReplicaCommit {
                protocol_version: PROTOCOL_VERSION,
                view: validator::ViewNumber(header.number.0),
                proposal: header,
            });
            let justification = validator::CommitQC::from(&[commit], validators).unwrap();
            Some(FinalBlock::new(header, payload, justification))
        });
        blocks.skip(1).take(count).collect()
    }

    async fn into_executor(
        self,
        storage: Arc<InMemoryStorage>,
    ) -> Executor<InMemoryStorage> {
        let mut executor = Executor::new(self.node_config, self.node_key, storage.clone())
            .unwrap();
        executor
            .set_validator(
                self.consensus_config,
                self.validator_key,
                storage,
                Arc::new(RandomPayloadSource),
            )
            .unwrap();
        executor
    }
}

type BlockMutation = (&'static str, fn(&mut FinalBlock));
const BLOCK_MUTATIONS: [BlockMutation; 3] = [
    ("number", |block| {
        block.header.number = BlockNumber(1);
    }),
    ("payload", |block| {
        block.payload = Payload(b"test".to_vec());
    }),
    ("justification", |block| {
        block.justification = thread_rng().gen();
    }),
];

#[tokio::test]
async fn genesis_block_mismatch() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let validator = FullValidatorConfig::for_single_validator(rng, Payload(vec![]), BlockNumber(0));
    let mut genesis_block = validator.genesis_block.clone();
    genesis_block.header.number = BlockNumber(1);
    let storage = Arc::new(InMemoryStorage::new(genesis_block.clone()));
    let err = Executor::new(validator.node_config, validator.node_key, storage)
        .err()
        .unwrap();
    tracing::info!(%err, "received expected validation error");
}

#[tokio::test]
async fn executing_single_validator() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let validator = FullValidatorConfig::for_single_validator(rng, Payload(vec![]), BlockNumber(0));
    let genesis_block = &validator.genesis_block;
    let storage = InMemoryStorage::new(genesis_block.clone());
    let storage = Arc::new(storage);
    let executor = validator.into_executor(ctx, storage.clone()).await;

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
async fn executing_validator_and_full_node() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let mut validator =
        FullValidatorConfig::for_single_validator(rng, Payload(vec![]), BlockNumber(0));
    let full_node = validator.connect_full_node(rng);

    let genesis_block = &validator.node_config.genesis_block;
    let validator_storage = InMemoryStorage::new(genesis_block.clone());
    let validator_storage = Arc::new(validator_storage);
    let full_node_storage = InMemoryStorage::new(genesis_block.clone());
    let full_node_storage = Arc::new(full_node_storage);
    let mut full_node_subscriber = full_node_storage.subscribe_to_block_writes();

    let validator = validator
        .into_executor(ctx, validator_storage.clone())
        .await;
    let full_node = Executor::new(
        ctx,
        full_node.node_config,
        full_node.node_key,
        full_node_storage.clone(),
    )
    .await
    .unwrap();

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(validator.run(ctx));
        s.spawn_bg(full_node.run(ctx));
        for _ in 0..5 {
            let number = *sync::changed(ctx, &mut full_node_subscriber).await?;
            tracing::trace!(%number, "Full node received block");
        }
        anyhow::Ok(())
    })
    .await
    .unwrap();
}

#[test_casing(2, [false, true])]
#[tokio::test]
async fn syncing_full_node_from_snapshot(delay_block_storage: bool) {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let mut validator =
        FullValidatorConfig::for_single_validator(rng, Payload(vec![]), BlockNumber(0));
    let mut full_node = validator.connect_full_node(rng);

    let genesis_block = &validator.genesis_block;
    let blocks = validator.gen_blocks(rng, 10);
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

    // Start a full node from a snapshot.
    full_node.genesis_block = blocks[3].clone();
    let full_node_storage = InMemoryStorage::new(blocks[3].clone());
    let full_node_storage = Arc::new(full_node_storage);
    let mut full_node_subscriber = full_node_storage.subscribe_to_block_writes();

    let full_node = Executor::new(
        full_node.node_config,
        full_node.node_key,
        full_node_storage.clone(),
    )
    .unwrap();

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(validator.run(ctx));
        s.spawn_bg(full_node.run(ctx));

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
            let last_contiguous_full_node_block =
                full_node_storage.last_contiguous_block_number(ctx).await?;
            tracing::trace!(
                %last_contiguous_full_node_block,
                "Full node updated last contiguous block"
            );
            if last_contiguous_full_node_block == BlockNumber(10) {
                break; // The full node has received all blocks!
            }
            // Wait until the node storage is updated.
            let number = *sync::changed(ctx, &mut full_node_subscriber).await?;
            tracing::trace!(%number, "Full node received block");
        }

        // Check that the node didn't receive any blocks with number lesser than the initial snapshot block.
        for lesser_block_number in 0..3 {
            let block = full_node_storage
                .block(ctx, BlockNumber(lesser_block_number))
                .await?;
            assert!(block.is_none());
        }
        anyhow::Ok(())
    })
    .await
    .unwrap();
}
