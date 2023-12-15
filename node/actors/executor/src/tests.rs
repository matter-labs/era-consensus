//! High-level tests for `Executor`.

use super::*;
use crate::testonly::{ValidatorNode};
use rand::{Rng};
use std::iter;
use test_casing::test_casing;
use zksync_concurrency::{sync, testonly::abort_on_panic, time};
use zksync_consensus_bft::{testonly, PROTOCOL_VERSION};
use zksync_consensus_roles::validator::{BlockNumber, FinalBlock, Payload};
use zksync_consensus_storage::{BlockStore, InMemoryStorage};

impl Config {
    fn into_executor(self, storage: Arc<InMemoryStorage>) -> Executor {
        Executor { config: self, storage, validator: None}
    }
}

impl ValidatorNode {
    fn gen_blocks<'a>(&'a self, rng: &'a mut impl Rng) -> impl 'a + Iterator<Item=FinalBlock> {
        let (genesis_block,_) = testonly::make_genesis(&[self.validator.key.clone()], Payload(vec![]), BlockNumber(0));
        let validators = &self.node.validators;
        iter::successors(Some(genesis_block), |parent| {
            let payload: Payload = rng.gen();
            let header = validator::BlockHeader {
                parent: parent.header.hash(),
                number: parent.header.number.next(),
                payload: payload.hash(),
            };
            let commit = self.validator.key.sign_msg(validator::ReplicaCommit {
                protocol_version: PROTOCOL_VERSION,
                view: validator::ViewNumber(header.number.0),
                proposal: header,
            });
            let justification = validator::CommitQC::from(&[commit], validators).unwrap();
            Some(FinalBlock::new(header, payload, justification))
        })
    }

    fn into_executor(self, storage: Arc<InMemoryStorage>) -> Executor {
        Executor {
            config: self.node, 
            storage: storage.clone(),
            validator: Some(Validator {
                config: self.validator,
                replica_state_store: storage,
                payload_source: Arc::new(testonly::RandomPayloadSource),
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
async fn executing_validator_and_full_node() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let mut validator = ValidatorNode::for_single_validator(rng);
    let full_node = validator.connect_full_node(rng);

    let genesis_block = validator.gen_blocks(rng).next().unwrap(); 
    let validator_storage = Arc::new(InMemoryStorage::new(genesis_block.clone()));
    let full_node_storage = Arc::new(InMemoryStorage::new(genesis_block.clone()));
    let mut full_node_subscriber = full_node_storage.subscribe_to_block_writes();

    let validator = validator.into_executor(validator_storage.clone());
    let full_node = full_node.into_executor(full_node_storage.clone());

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

    let mut validator = ValidatorNode::for_single_validator(rng);
    let full_node = validator.connect_full_node(rng);

    let blocks : Vec<_> = validator.gen_blocks(rng).take(11).collect();
    let validator_storage = InMemoryStorage::new(blocks[0].clone());
    let validator_storage = Arc::new(validator_storage);
    if !delay_block_storage {
        // Instead of running consensus on the validator, add the generated blocks manually.
        for block in &blocks {
            validator_storage.put_block(ctx, block).await.unwrap();
        }
    }
    let validator = validator.node.into_executor(validator_storage.clone());

    // Start a full node from a snapshot.
    let full_node_storage = InMemoryStorage::new(blocks[4].clone());
    let full_node_storage = Arc::new(full_node_storage);
    let mut full_node_subscriber = full_node_storage.subscribe_to_block_writes();

    let full_node = Executor {
        config: full_node,
        storage: full_node_storage.clone(),
        validator: None,
    };

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(validator.run(ctx));
        s.spawn_bg(full_node.run(ctx));

        if delay_block_storage {
            // Emulate the validator gradually adding new blocks to the storage.
            s.spawn_bg(async {
                for block in &blocks[1..] {
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
