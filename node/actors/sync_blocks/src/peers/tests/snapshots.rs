//! Tests related to snapshot storage.

use super::*;
use zksync_consensus_network::io::GetBlockError;

#[derive(Debug)]
struct UpdatingPeerStateWithStorageSnapshot;

#[async_trait]
impl Test for UpdatingPeerStateWithStorageSnapshot {
    const BLOCK_COUNT: usize = 5;
    const GENESIS_BLOCK_NUMBER: usize = 2;

    fn tweak_config(&self, config: &mut Config) {
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            mut rng,
            test_validators,
            peer_states_handle,
            storage,
            mut message_receiver,
            mut events_receiver,
            clock,
        } = handles;
        let mut storage_subscriber = storage.subscribe_to_block_writes();

        let peer_key = rng.gen::<node::SecretKey>().public();
        for stale_block_number in [1, 2] {
            peer_states_handle.update(
                peer_key.clone(),
                test_validators.sync_state(stale_block_number),
            );
            let peer_event = events_receiver.recv(ctx).await?;
            assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

            // No new block requests should be issued.
            clock.advance(BLOCK_SLEEP_INTERVAL);
            sync::yield_now().await;
            assert!(message_receiver.try_recv().is_none());
        }

        peer_states_handle.update(peer_key.clone(), test_validators.sync_state(3));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

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
        test_validators.send_block(BlockNumber(3), response);

        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::GotBlock(BlockNumber(3)));

        // Check that the block has been saved locally.
        let saved_block = *sync::changed(ctx, &mut storage_subscriber).await?;
        assert_eq!(saved_block, BlockNumber(3));
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

    fn tweak_config(&self, config: &mut Config) {
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            mut rng,
            test_validators,
            peer_states_handle,
            mut message_receiver,
            mut events_receiver,
            clock,
            ..
        } = handles;

        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states_handle.update(peer_key.clone(), test_validators.snapshot_sync_state(2..=2));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

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
        test_validators.send_block(BlockNumber(2), response);
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::GotBlock(BlockNumber(2)));

        // No further requests should be made.
        clock.advance(BLOCK_SLEEP_INTERVAL);
        sync::yield_now().await;
        assert!(message_receiver.try_recv().is_none());

        // Emulate peer receiving / producing a new block.
        peer_states_handle.update(peer_key.clone(), test_validators.snapshot_sync_state(2..=3));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

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
        peer_states_handle.update(full_peer_key.clone(), test_validators.sync_state(3));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == full_peer_key);

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

        test_validators.send_block(BlockNumber(1), response);
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::GotBlock(BlockNumber(1)));

        drop(block3_response); // Emulate first peer disconnecting.
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerDisconnected(key) if key == peer_key);
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

    fn tweak_config(&self, config: &mut Config) {
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            mut rng,
            test_validators,
            peer_states_handle,
            mut message_receiver,
            mut events_receiver,
            clock,
            ..
        } = handles;

        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states_handle.update(peer_key.clone(), test_validators.sync_state(1));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response: block1_response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(1));

        // Emulate peer pruning blocks.
        peer_states_handle.update(peer_key.clone(), test_validators.snapshot_sync_state(3..=3));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(3));

        test_validators.send_block(BlockNumber(3), response);
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::GotBlock(BlockNumber(3)));

        // No new blocks should be requested (the peer has no block #2).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        sync::yield_now().await;
        assert!(message_receiver.try_recv().is_none());

        block1_response.send(Err(GetBlockError::NotSynced)).unwrap();
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

    fn tweak_config(&self, config: &mut Config) {
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            mut rng,
            test_validators,
            peer_states_handle,
            mut message_receiver,
            mut events_receiver,
            clock,
            ..
        } = handles;

        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states_handle.update(peer_key.clone(), test_validators.snapshot_sync_state(3..=3));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient, number, ..
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(3));

        peer_states_handle.update(peer_key.clone(), test_validators.sync_state(3));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

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
