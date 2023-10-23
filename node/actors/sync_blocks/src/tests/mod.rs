//! Tests for the block syncing actor.

use super::*;
use concurrency::{oneshot, time};
use network::io::{GetBlockError, GetBlockResponse, SyncBlocksRequest};
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use roles::validator::{self, Block, BlockNumber, CommitQC, FinalBlock, ValidatorSet};
use std::iter;
use storage::RocksdbStorage;
use utils::pipe;

mod end_to_end;

const TEST_TIMEOUT: time::Duration = time::Duration::seconds(20);

impl Distribution<Config> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Config {
        let validator_set: ValidatorSet = rng.gen();
        let consensus_threshold = validator_set.len();
        Config::new(validator_set, consensus_threshold).unwrap()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TestValidators {
    validator_secret_keys: Vec<validator::SecretKey>,
    pub(crate) validator_set: ValidatorSet,
    pub(crate) final_blocks: Vec<FinalBlock>,
}

impl TestValidators {
    pub(crate) fn new(validator_count: usize, block_count: usize, rng: &mut impl Rng) -> Self {
        let validator_secret_keys: Vec<validator::SecretKey> =
            (0..validator_count).map(|_| rng.gen()).collect();
        let validator_set = validator_secret_keys.iter().map(|sk| sk.public());
        let validator_set = validator::ValidatorSet::new(validator_set).unwrap();

        let mut this = Self {
            validator_secret_keys,
            validator_set,
            final_blocks: vec![],
        };

        let mut latest_block = Block::genesis(vec![]);
        let final_blocks = (0..block_count).map(|_| {
            let final_block = FinalBlock {
                block: latest_block.clone(),
                justification: this.certify_block(&latest_block),
            };
            latest_block = Block {
                parent: latest_block.hash(),
                number: latest_block.number.next(),
                payload: vec![],
            };
            final_block
        });
        this.final_blocks = final_blocks.collect();
        this
    }

    pub(crate) fn test_config(&self) -> Config {
        Config::new(self.validator_set.clone(), self.validator_secret_keys.len()).unwrap()
    }

    fn certify_block(&self, block: &Block) -> CommitQC {
        let message_to_sign = validator::ReplicaCommit {
            view: validator::ViewNumber(block.number.0),
            proposal_block_hash: block.hash(),
            proposal_block_number: block.number,
        };
        let signed_messages: Vec<_> = self
            .validator_secret_keys
            .iter()
            .map(|sk| sk.sign_msg(message_to_sign))
            .collect();
        CommitQC::from(&signed_messages, &self.validator_set).unwrap()
    }

    pub(crate) fn sync_state(&self, last_block_number: usize) -> SyncState {
        let first_block = self.final_blocks[0].justification.clone();
        let last_block = self.final_blocks[last_block_number].justification.clone();
        SyncState {
            first_stored_block: first_block,
            last_contiguous_stored_block: last_block.clone(),
            last_stored_block: last_block,
        }
    }

    pub(crate) fn send_block(
        &self,
        number: BlockNumber,
        response: oneshot::Sender<GetBlockResponse>,
    ) {
        let final_block = self.final_blocks[number.0 as usize].clone();
        response.send(Ok(final_block)).unwrap();
        tracing::trace!("Responded to get_block({number})");
    }
}

#[tokio::test]
async fn subscribing_to_state_updates() {
    concurrency::testonly::abort_on_panic();

    let storage_dir = tempfile::tempdir().unwrap();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let genesis_block = create_block_from_base(Block::genesis(vec![]), rng);
    let block_1 = create_block(&genesis_block, rng);
    let block_2 = create_block(&block_1, rng);
    let block_3 = create_block(&block_2, rng);

    let storage = RocksdbStorage::new(ctx, &genesis_block, storage_dir.path()).await;
    let storage = &Arc::new(storage.unwrap());
    let (actor_pipe, _dispatcher_pipe) = pipe::new();
    let actor = SyncBlocks::new(ctx, actor_pipe, storage.clone(), rng.gen())
        .await
        .unwrap();
    let mut state_subscriber = actor.subscribe_to_state_updates();

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(async {
            actor.run(ctx).await.or_else(|err| {
                if err.root_cause().is::<ctx::Canceled>() {
                    Ok(()) // Swallow cancellation errors after the test is finished
                } else {
                    Err(err)
                }
            })
        });
        s.spawn_bg(async {
            assert!(ctx.sleep(TEST_TIMEOUT).await.is_err(), "Test timed out");
            anyhow::Ok(())
        });

        let initial_state = state_subscriber.borrow_and_update();
        assert_eq!(
            initial_state.first_stored_block,
            genesis_block.justification
        );
        assert_eq!(
            initial_state.last_contiguous_stored_block,
            genesis_block.justification
        );
        assert_eq!(initial_state.last_stored_block, genesis_block.justification);
        drop(initial_state);

        storage.put_block(ctx, &block_1).await.unwrap();

        let new_state = sync::changed(ctx, &mut state_subscriber).await?;
        assert_eq!(new_state.first_stored_block, genesis_block.justification);
        assert_eq!(
            new_state.last_contiguous_stored_block,
            block_1.justification
        );
        assert_eq!(new_state.last_stored_block, block_1.justification);
        drop(new_state);

        storage.put_block(ctx, &block_3).await.unwrap();

        let new_state = sync::changed(ctx, &mut state_subscriber).await?;
        assert_eq!(new_state.first_stored_block, genesis_block.justification);
        assert_eq!(
            new_state.last_contiguous_stored_block,
            block_1.justification
        );
        assert_eq!(new_state.last_stored_block, block_3.justification);

        Ok(())
    })
    .await
    .unwrap();
}

fn create_block(parent: &FinalBlock, rng: &mut impl Rng) -> FinalBlock {
    let block = Block {
        parent: parent.block.hash(),
        number: parent.block.number.next(),
        payload: Vec::new(),
    };
    create_block_from_base(block, rng)
}

fn create_block_from_base(block: Block, rng: &mut impl Rng) -> FinalBlock {
    let mut justification: CommitQC = rng.gen();
    justification.message.proposal_block_number = block.number;
    justification.message.proposal_block_hash = block.hash();

    FinalBlock {
        block,
        justification,
    }
}

#[tokio::test]
async fn getting_blocks() {
    concurrency::testonly::abort_on_panic();

    let storage_dir = tempfile::tempdir().unwrap();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let genesis_block = create_block_from_base(Block::genesis(vec![]), rng);

    let storage = RocksdbStorage::new(ctx, &genesis_block, storage_dir.path());
    let storage = Arc::new(storage.await.unwrap());
    let blocks = iter::successors(Some(genesis_block), |parent| {
        Some(create_block(parent, rng))
    });
    let blocks: Vec<_> = blocks.take(5).collect();
    for block in &blocks {
        storage.put_block(ctx, block).await.unwrap();
    }

    let (actor_pipe, dispatcher_pipe) = pipe::new();
    let actor = SyncBlocks::new(ctx, actor_pipe, storage.clone(), rng.gen())
        .await
        .unwrap();

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(async {
            actor.run(ctx).await.or_else(|err| {
                if err.root_cause().is::<ctx::Canceled>() {
                    Ok(()) // Swallow cancellation errors after the test is finished
                } else {
                    Err(err)
                }
            })
        });
        s.spawn_bg(async {
            assert!(ctx.sleep(TEST_TIMEOUT).await.is_err(), "Test timed out");
            anyhow::Ok(())
        });

        for (i, expected_block) in blocks.iter().enumerate() {
            let (response, response_receiver) = oneshot::channel();
            let input_message = SyncBlocksRequest::GetBlock {
                block_number: BlockNumber(i as u64),
                response,
            };
            dispatcher_pipe.send.send(input_message.into());
            let block = response_receiver.recv_or_disconnected(ctx).await???;
            assert_eq!(block, *expected_block);
        }

        let (response, response_receiver) = oneshot::channel();
        let input_message = SyncBlocksRequest::GetBlock {
            block_number: BlockNumber(777),
            response,
        };
        dispatcher_pipe.send.send(input_message.into());
        let block_err = response_receiver
            .recv_or_disconnected(ctx)
            .await??
            .unwrap_err();
        assert!(matches!(block_err, GetBlockError::NotSynced));

        Ok(())
    })
    .await
    .unwrap();
}
