//! Tests for the block syncing actor.
use super::*;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::{iter, ops};
use zksync_concurrency::{oneshot, sync, testonly::abort_on_panic, time};
use zksync_consensus_network::io::{GetBlockError, GetBlockResponse, SyncBlocksRequest};
use zksync_consensus_roles::validator::{
    self,
    testonly::{make_block, make_genesis_block},
    BlockHeader, BlockNumber, CommitQC, FinalBlock, Payload, ValidatorSet,
};
use zksync_consensus_storage::{testonly::in_memory, BlockStore, BlockStoreRunner};
use zksync_consensus_utils::pipe;

mod end_to_end;

const TEST_TIMEOUT: time::Duration = time::Duration::seconds(20);

pub(crate) async fn make_store(
    ctx: &ctx::Ctx,
    genesis: FinalBlock,
) -> (Arc<BlockStore>, BlockStoreRunner) {
    let storage = in_memory::BlockStore::new(genesis);
    BlockStore::new(ctx, Box::new(storage)).await.unwrap()
}

pub(crate) async fn wait_for_stored_block(
    ctx: &ctx::Ctx,
    storage: &BlockStore,
    block_number: BlockNumber,
) -> ctx::OrCanceled<()> {
    tracing::trace!("Started waiting for stored block");
    sync::wait_for(ctx, &mut storage.subscribe(), |state| {
        state.next() > block_number
    })
    .await?;
    Ok(())
}

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
    pub(crate) fn new(rng: &mut impl Rng, validator_count: usize, block_count: usize) -> Self {
        let validator_secret_keys: Vec<validator::SecretKey> =
            (0..validator_count).map(|_| rng.gen()).collect();
        let validator_set = validator_secret_keys.iter().map(|sk| sk.public());
        let validator_set = ValidatorSet::new(validator_set).unwrap();

        let mut this = Self {
            validator_secret_keys,
            validator_set,
            final_blocks: vec![],
        };

        let payload = Payload(vec![]);
        let mut latest_block = BlockHeader::genesis(payload.hash(), BlockNumber(0));
        let final_blocks = (0..block_count).map(|_| {
            let final_block = FinalBlock {
                payload: payload.clone(),
                justification: this.certify_block(&latest_block),
            };
            latest_block = BlockHeader::new(&latest_block, payload.hash());
            final_block
        });
        this.final_blocks = final_blocks.collect();
        this
    }

    pub(crate) fn test_config(&self) -> Config {
        Config::new(self.validator_set.clone(), self.validator_secret_keys.len()).unwrap()
    }

    fn certify_block(&self, proposal: &BlockHeader) -> CommitQC {
        let message_to_sign = validator::ReplicaCommit {
            protocol_version: validator::ProtocolVersion::EARLIEST,
            view: validator::ViewNumber(proposal.number.0),
            proposal: *proposal,
        };
        let signed_messages: Vec<_> = self
            .validator_secret_keys
            .iter()
            .map(|sk| sk.sign_msg(message_to_sign))
            .collect();
        CommitQC::from(&signed_messages, &self.validator_set).unwrap()
    }

    pub(crate) fn sync_state(&self, last_block_number: usize) -> BlockStoreState {
        self.snapshot_sync_state(1..=last_block_number)
    }

    pub(crate) fn snapshot_sync_state(
        &self,
        block_numbers: ops::RangeInclusive<usize>,
    ) -> BlockStoreState {
        assert!(!block_numbers.is_empty());
        BlockStoreState {
            first: self.final_blocks[*block_numbers.start()]
                .justification
                .clone(),
            last: self.final_blocks[*block_numbers.end()]
                .justification
                .clone(),
        }
    }

    pub(crate) fn send_block(
        &self,
        number: BlockNumber,
        response: oneshot::Sender<GetBlockResponse>,
    ) {
        let final_block = self.final_blocks[number.0 as usize].clone();
        match response.send(Ok(final_block)) {
            Ok(()) => tracing::info!(?number, "responded to get_block()"),
            Err(_) => tracing::info!(?number, "failed to respond to get_block()"),
        }
    }
}

#[tokio::test]
async fn subscribing_to_state_updates() {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let protocol_version = validator::ProtocolVersion::EARLIEST;
    let genesis_block = make_genesis_block(rng, protocol_version);
    let block_1 = make_block(rng, genesis_block.header(), protocol_version);

    let (storage, runner) = make_store(ctx, genesis_block.clone()).await;
    let (actor_pipe, _dispatcher_pipe) = pipe::new();
    let mut state_subscriber = storage.subscribe();

    let cfg: Config = rng.gen();
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(cfg.run(ctx, actor_pipe, storage.clone()));
        s.spawn_bg(async {
            assert!(ctx.sleep(TEST_TIMEOUT).await.is_err(), "Test timed out");
            anyhow::Ok(())
        });

        let state = state_subscriber.borrow().clone();
        assert_eq!(state.first, genesis_block.justification);
        assert_eq!(state.last, genesis_block.justification);
        storage.store_block(ctx, block_1.clone()).await.unwrap();

        let state = sync::wait_for(ctx, &mut state_subscriber, |state| {
            state.next() > block_1.header().number
        })
        .await
        .unwrap()
        .clone();
        assert_eq!(state.first, genesis_block.justification);
        assert_eq!(state.last, block_1.justification);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn getting_blocks() {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let protocol_version = validator::ProtocolVersion::EARLIEST;
    let genesis_block = make_genesis_block(rng, protocol_version);

    let (storage, runner) = make_store(ctx, genesis_block.clone()).await;
    let (actor_pipe, dispatcher_pipe) = pipe::new();

    let cfg: Config = rng.gen();
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(runner.run(ctx));
        let blocks = iter::successors(Some(genesis_block), |parent| {
            Some(make_block(rng, parent.header(), protocol_version))
        });
        let blocks: Vec<_> = blocks.take(5).collect();
        for block in &blocks {
            storage.store_block(ctx, block.clone()).await.unwrap();
        }
        s.spawn_bg(cfg.run(ctx, actor_pipe, storage.clone()));
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
