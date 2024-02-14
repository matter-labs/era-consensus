//! Basic tests.

use super::*;
use crate::{
    io,
    tests::{send_block, sync_state},
};

#[derive(Debug)]
struct UpdatingPeerStateWithSingleBlock;

#[async_trait]
impl Test for UpdatingPeerStateWithSingleBlock {
    const BLOCK_COUNT: usize = 2;

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            setup,
            peer_states,
            storage,
            mut message_receiver,
            mut events_receiver,
            ..
        } = handles;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&peer_key, sync_state(&setup, BlockNumber(1)))
            .unwrap();

        // Check that the actor has sent a `get_block` request to the peer
        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(1));

        // Emulate the peer sending a correct response.
        send_block(&setup, BlockNumber(1), response);

        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::GotBlock(BlockNumber(1)));

        // Check that the block has been saved locally.
        storage.wait_until_persisted(ctx, BlockNumber(1)).await?;
        Ok(())
    }
}

#[tokio::test]
async fn updating_peer_state_with_single_block() {
    test_peer_states(UpdatingPeerStateWithSingleBlock).await;
}

#[derive(Debug)]
struct CancelingBlockRetrieval;

#[async_trait]
impl Test for CancelingBlockRetrieval {
    const BLOCK_COUNT: usize = 5;

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            setup,
            peer_states,
            storage,
            mut message_receiver,
            ..
        } = handles;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&peer_key, sync_state(&setup, BlockNumber(1)))
            .unwrap();

        // Check that the actor has sent a `get_block` request to the peer
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock { mut response, .. }) =
            message_receiver.recv(ctx).await?;

        // Emulate receiving block using external means.
        storage.queue_block(ctx, setup.blocks[1].clone()).await?;

        // Retrieval of the block must be canceled.
        response.closed().await;
        Ok(())
    }
}

#[tokio::test]
async fn canceling_block_retrieval() {
    test_peer_states(CancelingBlockRetrieval).await;
}

#[derive(Debug)]
struct FilteringBlockRetrieval;

#[async_trait]
impl Test for FilteringBlockRetrieval {
    const BLOCK_COUNT: usize = 5;

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            setup,
            peer_states,
            storage,
            mut message_receiver,
            ..
        } = handles;

        // Emulate receiving block using external means.
        storage.queue_block(ctx, setup.blocks[1].clone()).await?;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&peer_key, sync_state(&setup, BlockNumber(2)))
            .unwrap();

        // Check that the actor has sent `get_block` request to the peer, but only for block #2.
        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient, number, ..
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(2));

        assert!(message_receiver.try_recv().is_none());
        Ok(())
    }
}

#[tokio::test]
async fn filtering_block_retrieval() {
    test_peer_states(FilteringBlockRetrieval).await;
}

#[derive(Debug)]
struct UpdatingPeerStateWithMultipleBlocks;

impl UpdatingPeerStateWithMultipleBlocks {
    const MAX_CONCURRENT_BLOCKS: usize = 3;
}

#[async_trait]
impl Test for UpdatingPeerStateWithMultipleBlocks {
    const BLOCK_COUNT: usize = 10;

    fn config(&self, setup: &validator::testonly::GenesisSetup) -> Config {
        let mut config = Config::new(setup.genesis.clone());
        config.max_concurrent_blocks_per_peer = Self::MAX_CONCURRENT_BLOCKS;
        // ^ We want to test rate limiting for peers
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            clock,
            setup,
            peer_states,
            storage,
            mut message_receiver,
            mut events_receiver,
        } = handles;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&peer_key, sync_state(&setup, BlockNumber((Self::BLOCK_COUNT - 1) as u64)).clone())
            .unwrap();

        let mut requested_blocks = HashMap::with_capacity(Self::MAX_CONCURRENT_BLOCKS);
        for _ in 1..Self::BLOCK_COUNT {
            let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                recipient,
                number,
                response,
            }) = message_receiver.recv(ctx).await.unwrap();

            tracing::trace!("Received request for block #{number}");
            assert_eq!(recipient, peer_key);
            assert!(
                requested_blocks.insert(number, response).is_none(),
                "Block #{number} requested twice"
            );

            if requested_blocks.len() == Self::MAX_CONCURRENT_BLOCKS || rng.gen() {
                // Answer a random request.
                let number = *requested_blocks.keys().choose(rng).unwrap();
                let response = requested_blocks.remove(&number).unwrap();
                send_block(&setup, number, response);

                let peer_event = events_receiver.recv(ctx).await?;
                assert_matches!(peer_event, PeerStateEvent::GotBlock(got) if got == number);
            }
            clock.advance(BLOCK_SLEEP_INTERVAL);
        }

        // Answer all remaining requests.
        for (number, response) in requested_blocks {
            send_block(&setup, number, response);
            let peer_event = events_receiver.recv(ctx).await?;
            assert_matches!(peer_event, PeerStateEvent::GotBlock(got) if got == number);
        }

        let expected_block_number = BlockNumber(Self::BLOCK_COUNT as u64 - 1);
        storage
            .wait_until_persisted(ctx, expected_block_number)
            .await?;
        Ok(())
    }
}

#[tokio::test]
async fn updating_peer_state_with_multiple_blocks() {
    test_peer_states(UpdatingPeerStateWithMultipleBlocks).await;
}

#[derive(Debug)]
struct DisconnectingPeer;

#[async_trait]
impl Test for DisconnectingPeer {
    const BLOCK_COUNT: usize = 5;

    fn config(&self, setup: &validator::testonly::GenesisSetup) -> Config {
        let mut config = Config::new(setup.genesis.clone());
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            clock,
            setup,
            peer_states,
            storage,
            mut message_receiver,
            mut events_receiver,
        } = handles;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&peer_key, sync_state(&setup, BlockNumber(1)))
            .unwrap();

        // Drop the response sender emulating peer disconnect.
        let msg = message_receiver.recv(ctx).await?;
        {
            let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                recipient,
                number,
                ..
            }) = &msg;
            assert_eq!(recipient, &peer_key);
            assert_eq!(number, &BlockNumber(1));
        }
        drop(msg);

        wait_for_event(
            ctx,
            &mut events_receiver,
            |ev| matches!(ev, PeerStateEvent::PeerDropped(key) if key == peer_key),
        )
        .await
        .context("wait for PeerDropped")?;

        // Check that no new requests are sent (there are no peers to send them to).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert_matches!(message_receiver.try_recv(), None);

        // Re-connect the peer with an updated state.
        peer_states
            .update(&peer_key, sync_state(&setup, BlockNumber(2)))
            .unwrap();
        // Ensure that blocks are re-requested.
        clock.advance(BLOCK_SLEEP_INTERVAL);

        let mut responses = HashMap::with_capacity(2);
        for _ in 0..2 {
            let message = message_receiver.recv(ctx).await?;
            let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                recipient,
                number,
                response,
            }) = message;
            assert_eq!(recipient, peer_key);
            assert!(responses.insert(number.0, response).is_none());
        }

        assert!(responses.contains_key(&1));
        assert!(responses.contains_key(&2));
        // Send one of the responses and drop the other request.
        let response = responses.remove(&2).unwrap();
        send_block(&setup, BlockNumber(2), response);

        wait_for_event(ctx, &mut events_receiver, |ev| {
            matches!(ev, PeerStateEvent::GotBlock(BlockNumber(2)))
        })
        .await?;
        drop(responses);
        wait_for_event(
            ctx,
            &mut events_receiver,
            |ev| matches!(ev, PeerStateEvent::PeerDropped(key) if key == peer_key),
        )
        .await?;

        // Check that no new requests are sent (there are no peers to send them to).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert_matches!(message_receiver.try_recv(), None);

        // Re-connect the peer with the same state.
        peer_states
            .update(&peer_key, sync_state(&setup, BlockNumber(2)))
            .unwrap();
        clock.advance(BLOCK_SLEEP_INTERVAL);

        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(1));
        send_block(&setup, number, response);

        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::GotBlock(BlockNumber(1)));

        // Check that no new requests are sent (all blocks are downloaded).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert_matches!(message_receiver.try_recv(), None);

        storage.wait_until_persisted(ctx, BlockNumber(2)).await?;
        Ok(())
    }
}

#[tokio::test]
async fn disconnecting_peer() {
    test_peer_states(DisconnectingPeer).await;
}

#[derive(Debug)]
struct DownloadingBlocksInGaps {
    local_blocks: Vec<BlockNumber>,
    increase_peer_block_number_during_test: bool,
}

impl DownloadingBlocksInGaps {
    fn new(local_blocks: impl Iterator<Item=BlockNumber>) -> Self {
        Self {
            local_blocks: local_blocks
                .inspect(|&number| assert!(number.0 > 0 && (number.0 as usize) < Self::BLOCK_COUNT))
                .collect(),
            increase_peer_block_number_during_test: false,
        }
    }
}

#[async_trait]
impl Test for DownloadingBlocksInGaps {
    const BLOCK_COUNT: usize = 10;

    fn config(&self, setup: &validator::testonly::GenesisSetup) -> Config {
        let mut config = Config::new(setup.genesis.clone());
        config.max_concurrent_blocks = 1;
        // ^ Forces the node to download blocks in a deterministic order
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            clock,
            setup,
            peer_states,
            storage,
            mut message_receiver,
            ..
        } = handles;

        scope::run!(ctx, |ctx, s| async {
            for n in &self.local_blocks {
                s.spawn(storage.queue_block(ctx, setup.blocks[n.0 as usize].clone()));
            }
            let rng = &mut ctx.rng();
            let peer_key = rng.gen::<node::SecretKey>().public();
            let mut last_peer_block_number = BlockNumber(if self.increase_peer_block_number_during_test {
                rng.gen_range(0..setup.blocks.len())
            } else {
                setup.blocks.len()-1
            } as u64);
            peer_states
                .update(&peer_key, sync_state(&setup, last_peer_block_number))
                .unwrap();
            clock.advance(BLOCK_SLEEP_INTERVAL);

            let want : Vec<BlockNumber> = setup.blocks.iter().map(|b|b.header().number).filter(|n| !self.local_blocks.contains(n)).collect();

            // Check that all missing blocks are requested.
            for n in want {
                if n > last_peer_block_number {
                    last_peer_block_number = BlockNumber(rng.gen_range(n.0..Self::BLOCK_COUNT as u64));
                    peer_states.update(&peer_key, sync_state(&setup, last_peer_block_number)).unwrap();
                    clock.advance(BLOCK_SLEEP_INTERVAL);
                }

                let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                    recipient,
                    number,
                    response,
                }) = message_receiver.recv(ctx).await?;

                assert_eq!(recipient, peer_key);
                assert!(number <= last_peer_block_number);
                send_block(&setup, number, response);
                storage.wait_until_persisted(ctx, number).await?;
                clock.advance(BLOCK_SLEEP_INTERVAL);
            }
            Ok(())
        })
        .await?;
        Ok(())
    }
}

const LOCAL_BLOCK_NUMBERS: [&[u64]; 3] = [&[1, 9], &[3, 5, 6, 8], &[4]];

#[test_casing(6, Product((LOCAL_BLOCK_NUMBERS, [false, true])))]
#[tokio::test]
async fn downloading_blocks_in_gaps(
    local_blocks: &[u64],
    increase_peer_block_number_during_test: bool,
) {
    let mut test = DownloadingBlocksInGaps::new(local_blocks.iter().map(|n|BlockNumber(*n)));
    test.increase_peer_block_number_during_test = increase_peer_block_number_during_test;
    test_peer_states(test).await;
}

#[derive(Debug)]
struct LimitingGetBlockConcurrency;

#[async_trait]
impl Test for LimitingGetBlockConcurrency {
    const BLOCK_COUNT: usize = 5;

    fn config(&self, setup: &validator::testonly::GenesisSetup) -> Config {
        let mut config = Config::new(setup.genesis.clone());
        config.max_concurrent_blocks = 3;
        config
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            setup,
            peer_states,
            storage,
            mut message_receiver,
            ..
        } = handles;
        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&peer_key, sync_state(&setup, setup.blocks.last().unwrap().header().number))
            .unwrap();

        // The actor should request 3 new blocks it's now aware of from the only peer it's currently
        // aware of. Note that blocks may be queried in any order.
        let mut message_responses = HashMap::with_capacity(3);
        for _ in 0..3 {
            let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                recipient,
                number,
                response,
            }) = message_receiver.recv(ctx).await?;
            assert_eq!(recipient, peer_key);
            assert!(message_responses.insert(number.0, response).is_none());
        }
        assert_matches!(message_receiver.try_recv(), None);
        assert_eq!(
            message_responses.keys().copied().collect::<HashSet<_>>(),
            HashSet::from([1, 2, 3])
        );
        tracing::info!("blocks requrested");

        // Send a correct response.
        let response = message_responses.remove(&1).unwrap();
        send_block(&setup, BlockNumber(1), response);
        storage.wait_until_persisted(ctx, BlockNumber(1)).await?;

        // The actor should now request another block.
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient, number, ..
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(4));

        Ok(())
    }
}

#[tokio::test]
async fn limiting_get_block_concurrency() {
    test_peer_states(LimitingGetBlockConcurrency).await;
}
