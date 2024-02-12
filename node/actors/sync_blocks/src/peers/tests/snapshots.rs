//! Tests related to snapshot storage.

use super::*;
use crate::tests::{send_block, snapshot_sync_state, sync_state};
use zksync_consensus_network::io::GetBlockError;

#[derive(Debug)]
struct UpdatingPeerStateWithStorageSnapshot;

#[async_trait]
impl Test for UpdatingPeerStateWithStorageSnapshot {
    const BLOCK_COUNT: usize = 5;
    const GENESIS_BLOCK_NUMBER: usize = 2;

    fn config(&self, setup: &validator::testonly::GenesisSetup) -> Config {
        let mut config = Config::new(setup.genesis.clone());
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            setup,
            peer_states,
            storage,
            mut message_receiver,
            mut events_receiver,
            clock,
        } = handles;
        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        for stale_block_number in [1, 2] {
            peer_states
                .update(&peer_key, sync_state(&setup, stale_block_number))
                .unwrap();

            // No new block requests should be issued.
            clock.advance(BLOCK_SLEEP_INTERVAL);
            sync::yield_now().await;
            assert!(message_receiver.try_recv().is_none());
        }

        peer_states
            .update(&peer_key, sync_state(&setup, 3))
            .unwrap();

        // Check that the actor has sent a `get_block` request to the peer
        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(3));

        // Emulate the peer sending a correct response.
        send_block(&setup, BlockNumber(3), response);

        wait_for_event(ctx, &mut events_receiver, |ev| {
            matches!(ev, PeerStateEvent::GotBlock(BlockNumber(3)))
        })
        .await
        .unwrap();

        // Check that the block has been saved locally.
        storage.wait_until_queued(ctx, BlockNumber(3)).await?;
        Ok(())
    }
}

#[tokio::test]
async fn updating_peer_state_with_storage_snapshot() {
    test_peer_states(UpdatingPeerStateWithStorageSnapshot).await;
}

#[derive(Debug)]
struct FilteringRequestsForSnapshotPeer;

#[async_trait]
impl Test for FilteringRequestsForSnapshotPeer {
    const BLOCK_COUNT: usize = 5;

    fn config(&self, setup: &validator::testonly::GenesisSetup) -> Config {
        let mut config = Config::new(setup.genesis.clone());
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            setup,
            peer_states,
            mut message_receiver,
            mut events_receiver,
            clock,
            ..
        } = handles;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&peer_key, snapshot_sync_state(&setup, 2..=2))
            .unwrap();

        // The peer should only be queried for blocks that it actually has (#2 in this case).
        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(2));

        // Emulate the peer sending a correct response.
        send_block(&setup, BlockNumber(2), response);
        wait_for_event(ctx, &mut events_receiver, |ev| {
            matches!(ev, PeerStateEvent::GotBlock(BlockNumber(2)))
        })
        .await
        .unwrap();

        // No further requests should be made.
        clock.advance(BLOCK_SLEEP_INTERVAL);
        sync::yield_now().await;
        assert!(message_receiver.try_recv().is_none());

        // Emulate peer receiving / producing a new block.
        peer_states
            .update(&peer_key, snapshot_sync_state(&setup, 2..=3))
            .unwrap();

        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response: block3_response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(3));

        // Emulate another peer with full history.
        let full_peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&full_peer_key, sync_state(&setup, 3))
            .unwrap();
        clock.advance(BLOCK_SLEEP_INTERVAL);

        // A node should only request block #1 from the peer; block #3 is already requested,
        // and it has #2 locally.
        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response,
        }) = message;
        assert_eq!(recipient, full_peer_key);
        assert_eq!(number, BlockNumber(1));

        send_block(&setup, BlockNumber(1), response);
        wait_for_event(ctx, &mut events_receiver, |ev| {
            matches!(ev, PeerStateEvent::GotBlock(BlockNumber(1)))
        })
        .await
        .unwrap();

        drop(block3_response); // Emulate first peer disconnecting.
        wait_for_event(
            ctx,
            &mut events_receiver,
            |ev| matches!(ev,PeerStateEvent::PeerDropped(key) if key == peer_key),
        )
        .await
        .unwrap();
        clock.advance(BLOCK_SLEEP_INTERVAL);

        // Now, block #3 will be requested from the peer with full history.
        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient, number, ..
        }) = message;
        assert_eq!(recipient, full_peer_key);
        assert_eq!(number, BlockNumber(3));

        Ok(())
    }
}

#[tokio::test]
async fn filtering_requests_for_snapshot_peer() {
    test_peer_states(FilteringRequestsForSnapshotPeer).await;
}

#[derive(Debug)]
struct PruningPeerHistory;

#[async_trait]
impl Test for PruningPeerHistory {
    const BLOCK_COUNT: usize = 5;

    fn config(&self, setup: &validator::testonly::GenesisSetup) -> Config {
        let mut config = Config::new(setup.genesis.clone());
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            setup,
            peer_states,
            mut message_receiver,
            mut events_receiver,
            clock,
            ..
        } = handles;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&peer_key, sync_state(&setup, 1))
            .unwrap();

        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response: block1_response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(1));

        // Emulate peer pruning blocks.
        peer_states
            .update(&peer_key, snapshot_sync_state(&setup, 3..=3))
            .unwrap();

        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(3));

        send_block(&setup, BlockNumber(3), response);
        wait_for_event(ctx, &mut events_receiver, |ev| {
            matches!(ev, PeerStateEvent::GotBlock(BlockNumber(3)))
        })
        .await
        .unwrap();

        // No new blocks should be requested (the peer has no block #2).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        sync::yield_now().await;
        assert!(message_receiver.try_recv().is_none());

        block1_response
            .send(Err(GetBlockError::NotAvailable))
            .unwrap();
        // Block #1 should not be requested again (the peer no longer has it).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        sync::yield_now().await;
        assert!(message_receiver.try_recv().is_none());

        Ok(())
    }
}

#[tokio::test]
async fn pruning_peer_history() {
    test_peer_states(PruningPeerHistory).await;
}

#[derive(Debug)]
struct BackfillingPeerHistory;

#[async_trait]
impl Test for BackfillingPeerHistory {
    const BLOCK_COUNT: usize = 5;

    fn config(&self, setup: &validator::testonly::GenesisSetup) -> Config {
        let mut config = Config::new(setup.genesis.clone());
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            setup,
            peer_states,
            mut message_receiver,
            clock,
            ..
        } = handles;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&peer_key, snapshot_sync_state(&setup, 3..=3))
            .unwrap();

        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient, number, ..
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(3));

        peer_states
            .update(&peer_key, sync_state(&setup, 3))
            .unwrap();
        clock.advance(BLOCK_SLEEP_INTERVAL);
        let mut new_requested_numbers = HashSet::new();
        for _ in 0..2 {
            let message = message_receiver.recv(ctx).await?;
            let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                recipient,
                number,
                ..
            }) = message;
            assert_eq!(recipient, peer_key);
            new_requested_numbers.insert(number);
        }
        assert_eq!(
            new_requested_numbers,
            HashSet::from([BlockNumber(1), BlockNumber(2)])
        );

        Ok(())
    }
}

#[tokio::test]
async fn backfilling_peer_history() {
    test_peer_states(BackfillingPeerHistory).await;
}
