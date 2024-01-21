//! High-level tests for `Executor`.

use super::*;
use crate::testonly::{connect_full_node, ValidatorNode};
use test_casing::test_casing;
use zksync_concurrency::{sync, testonly::{abort_on_panic, set_timeout}, time};
use zksync_consensus_storage::testonly::new_store;
use zksync_consensus_bft as bft;
use zksync_consensus_roles::validator::{BlockNumber};
use zksync_consensus_storage::{testonly::in_memory, BlockStore};

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
    fn into_executor(self, block_store: Arc<BlockStore>) -> Executor {
        Executor {
            config: self.node,
            block_store,
            validator: Some(Validator {
                config: self.validator,
                replica_store: Box::new(in_memory::ReplicaStore::default()),
                payload_manager: Box::new(bft::testonly::RandomPayload(1000)),
            }),
        }
    }
}

#[tokio::test]
async fn executing_single_validator() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let validator = ValidatorNode::new(rng);
    let (storage, runner) = new_store(ctx, &validator.setup.blocks[0]).await;
    let executor = validator.into_executor(storage.clone());

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(runner.run(ctx));
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

    let mut validator = ValidatorNode::new(rng);
    let full_node = connect_full_node(rng, &mut validator.node);
    let (validator_storage, validator_runner) = new_store(ctx, &validator.setup.blocks[0]).await;
    let (full_node_storage, full_node_runner) = new_store(ctx, &validator.setup.blocks[0]).await;

    let validator = validator.into_executor(validator_storage.clone());
    let full_node = full_node.into_executor(full_node_storage.clone());

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(validator_runner.run(ctx));
        s.spawn_bg(full_node_runner.run(ctx));
        s.spawn_bg(validator.run(ctx));
        s.spawn_bg(full_node.run(ctx));
        full_node_storage.wait_until_persisted(ctx,BlockNumber(5)).await?;
        Ok(())
    })
    .await
    .unwrap();
}

#[test_casing(2, [false, true])]
#[tokio::test]
async fn syncing_full_node_from_snapshot(delay_block_storage: bool) {
    abort_on_panic();
    let _guard = set_timeout(time::Duration::seconds(10));

    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let mut validator = ValidatorNode::new(rng);
    validator.setup.push_blocks(rng,10);
    let node2 = connect_full_node(rng, &mut validator.node);

    let (store1, store1_runner) = new_store(ctx, &validator.setup.blocks[0]).await;
    // Node2 will start from a snapshot.
    let (store2, store2_runner) = new_store(ctx, &validator.setup.blocks[4]).await;
    
    // We spawn 2 non-validator nodes. We will simulate blocks appearing in storage of node1,
    // and will expect them to be propagated to node2.
    let node1 = validator.node.into_executor(store1.clone());
    let node2 = Executor {
        config: node2,
        block_store: store2.clone(),
        validator: None,
    };

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(store1_runner.run(ctx));
        s.spawn_bg(store2_runner.run(ctx));
        if !delay_block_storage {
            // Instead of running consensus on the validator, add the generated blocks manually.
            for block in &validator.setup.blocks[1..] {
                store1
                    .queue_block(ctx, block.clone())
                    .await
                    .unwrap();
            }
        }
        s.spawn_bg(node1.run(ctx));
        s.spawn_bg(node2.run(ctx));

        if delay_block_storage {
            // Emulate the validator gradually adding new blocks to the storage.
            s.spawn_bg(async {
                for block in &validator.setup.blocks[1..] {
                    ctx.sleep(time::Duration::milliseconds(500)).await?;
                    store1.queue_block(ctx, block.clone()).await?;
                }
                Ok(())
            });
        }

        store2.wait_until_persisted(ctx,BlockNumber(10)).await?;

        // Check that the node didn't receive any blocks with number lesser than the initial snapshot block.
        for lesser_block_number in 0..3 {
            let block = store2 
                .block(ctx, BlockNumber(lesser_block_number))
                .await?;
            assert!(block.is_none());
        }
        anyhow::Ok(())
    })
    .await
    .unwrap();
}
