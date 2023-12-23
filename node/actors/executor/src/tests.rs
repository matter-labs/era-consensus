//! High-level tests for `Executor`.

use super::*;
use crate::testonly::ValidatorNode;
use rand::Rng;
use std::iter;
use test_casing::test_casing;
use zksync_concurrency::{sync, testonly::abort_on_panic, time};
use zksync_consensus_bft::{testonly, PROTOCOL_VERSION};
use zksync_consensus_roles::validator::{BlockNumber, FinalBlock, Payload};
use zksync_consensus_storage::{BlockStore, testonly::in_memory};

async fn make_store(ctx:&ctx::Ctx, genesis: FinalBlock) -> Arc<BlockStore> {
   Arc::new(BlockStore::new(ctx,Box::new(in_memory::BlockStore::new(genesis)),10).await.unwrap())
}

impl Config {
    fn into_executor(self, block_store: Arc<BlockStore>) -> Executor {
        Executor {
            config: self,
            block_store,
            validator: None,
        }
    }
}

impl ValidatorNode {
    fn gen_blocks<'a>(&'a self, rng: &'a mut impl Rng) -> impl 'a + Iterator<Item = FinalBlock> {
        let (genesis_block, _) = testonly::make_genesis(
            &[self.validator.key.clone()],
            Payload(vec![]),
            BlockNumber(0),
        );
        let validators = &self.node.validators;
        iter::successors(Some(genesis_block), |parent| {
            let payload: Payload = rng.gen();
            let header = validator::BlockHeader {
                parent: parent.header().hash(),
                number: parent.header().number.next(),
                payload: payload.hash(),
            };
            let commit = self.validator.key.sign_msg(validator::ReplicaCommit {
                protocol_version: PROTOCOL_VERSION,
                view: validator::ViewNumber(header.number.0),
                proposal: header,
            });
            let justification = validator::CommitQC::from(&[commit], validators).unwrap();
            Some(FinalBlock::new(payload, justification))
        })
    }

    fn into_executor(self, block_store: Arc<BlockStore>) -> Executor {
        Executor {
            config: self.node,
            block_store,
            validator: Some(Validator {
                config: self.validator,
                replica_store: Box::new(in_memory::ReplicaStore::default()),
                payload_manager: Box::new(testonly::RandomPayload),
            }),
        }
    }
}

#[tokio::test]
async fn executing_single_validator() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let validator = ValidatorNode::for_single_validator(rng);
    let genesis_block = validator.gen_blocks(rng).next().unwrap();
    let storage = make_store(ctx,genesis_block.clone()).await;
    let executor = validator.into_executor(storage.clone());

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(storage.run_background_tasks(ctx));
        s.spawn_bg(executor.run(ctx));
        let want = BlockNumber(5);
        sync::wait_for(ctx, &mut storage.subscribe(), |state| state.next() > want).await?;
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

    let mut validator = ValidatorNode::for_single_validator(rng);
    let full_node = validator.connect_full_node(rng);

    let genesis_block = validator.gen_blocks(rng).next().unwrap();
    let validator_storage = make_store(ctx,genesis_block.clone()).await;
    let full_node_storage = make_store(ctx,genesis_block.clone()).await;

    let validator = validator.into_executor(validator_storage.clone());
    let full_node = full_node.into_executor(full_node_storage.clone());

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(validator_storage.run_background_tasks(ctx));
        s.spawn_bg(full_node_storage.run_background_tasks(ctx));
        s.spawn_bg(validator.run(ctx));
        s.spawn_bg(full_node.run(ctx));
        let want = BlockNumber(5);
        sync::wait_for(ctx, &mut full_node_storage.subscribe(), |state| state.next() > want).await?;
        Ok(())
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

    let mut validator = ValidatorNode::for_single_validator(rng);
    let full_node = validator.connect_full_node(rng);

    let blocks: Vec<_> = validator.gen_blocks(rng).take(11).collect();
    let validator_storage = make_store(ctx,blocks[0].clone()).await; 
    let validator = validator.node.into_executor(validator_storage.clone());

    // Start a full node from a snapshot.
    let full_node_storage = make_store(ctx,blocks[4].clone()).await;

    let full_node = Executor {
        config: full_node,
        block_store: full_node_storage.clone(),
        validator: None,
    };

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(validator_storage.run_background_tasks(ctx));
        s.spawn_bg(full_node_storage.run_background_tasks(ctx));
        if !delay_block_storage {
            // Instead of running consensus on the validator, add the generated blocks manually.
            for block in &blocks {
                validator_storage.store_block(ctx, block.clone()).await.unwrap();
            }
        }
        s.spawn_bg(validator.run(ctx));
        s.spawn_bg(full_node.run(ctx));

        if delay_block_storage {
            // Emulate the validator gradually adding new blocks to the storage.
            s.spawn_bg(async {
                for block in &blocks[1..] {
                    ctx.sleep(time::Duration::milliseconds(500)).await?;
                    validator_storage.store_block(ctx, block.clone()).await?;
                }
                Ok(())
            });
        }

        sync::wait_for(ctx, &mut full_node_storage.subscribe(), |state| {
            let last = state.last.header().number;
            tracing::trace!(%last, "Full node updated last block");
            last >= BlockNumber(10)
        }).await.unwrap();
           
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
