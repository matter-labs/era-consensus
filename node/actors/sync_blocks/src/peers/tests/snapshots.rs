//! Tests related to snapshot storage.

use super::*;

#[derive(Debug)]
struct UpdatingPeerStateWithStorageSnapshot;

#[async_trait]
impl Test for UpdatingPeerStateWithStorageSnapshot {
    const BLOCK_COUNT: usize = 5;
    const GENESIS_BLOCK_NUMBER: usize = 2;

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
