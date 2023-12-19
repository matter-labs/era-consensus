//! Tests focused on handling peers providing fake information to the node.

use super::*;

#[tokio::test]
async fn processing_invalid_sync_states() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let test_validators = TestValidators::new(4, 3, rng);
    let storage = InMemoryStorage::new(test_validators.final_blocks[0].clone());
    let storage = Arc::new(storage);

    let (message_sender, _) = channel::unbounded();
    let (peer_states, _) = PeerStates::new(message_sender, storage, test_validators.test_config());

    let mut invalid_sync_state = test_validators.sync_state(1);
    invalid_sync_state.first_stored_block = test_validators.final_blocks[2].justification.clone();
    assert!(peer_states.validate_sync_state(invalid_sync_state).is_err());

    let mut invalid_sync_state = test_validators.sync_state(1);
    invalid_sync_state.last_contiguous_stored_block =
        test_validators.final_blocks[2].justification.clone();
    assert!(peer_states.validate_sync_state(invalid_sync_state).is_err());

    let mut invalid_sync_state = test_validators.sync_state(1);
    invalid_sync_state
        .last_contiguous_stored_block
        .message
        .proposal
        .number = BlockNumber(5);
    invalid_sync_state.last_stored_block.message.proposal.number = BlockNumber(5);
    assert!(peer_states.validate_sync_state(invalid_sync_state).is_err());

    let other_network = TestValidators::new(4, 2, rng);
    let invalid_sync_state = other_network.sync_state(1);
    assert!(peer_states.validate_sync_state(invalid_sync_state).is_err());
}

#[tokio::test]
async fn processing_invalid_blocks() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let test_validators = TestValidators::new(4, 3, rng);
    let storage = InMemoryStorage::new(test_validators.final_blocks[0].clone());
    let storage = Arc::new(storage);

    let (message_sender, _) = channel::unbounded();
    let (peer_states, _) = PeerStates::new(message_sender, storage, test_validators.test_config());

    let invalid_block = &test_validators.final_blocks[0];
    let err = peer_states
        .validate_block(BlockNumber(1), invalid_block)
        .unwrap_err();
    assert_matches!(err, BlockValidationError::Other(_));

    let mut invalid_block = test_validators.final_blocks[1].clone();
    invalid_block.payload = validator::Payload(b"invalid".to_vec());
    let err = peer_states
        .validate_block(BlockNumber(1), &invalid_block)
        .unwrap_err();
    assert_matches!(err, BlockValidationError::HashMismatch { .. });

    let other_network = TestValidators::new(4, 2, rng);
    let invalid_block = &other_network.final_blocks[1];
    let err = peer_states
        .validate_block(BlockNumber(1), invalid_block)
        .unwrap_err();
    assert_matches!(err, BlockValidationError::Justification(_));
}

#[derive(Debug)]
struct PeerWithFakeSyncState;

#[async_trait]
impl Test for PeerWithFakeSyncState {
    const BLOCK_COUNT: usize = 10;

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            clock,
            mut rng,
            test_validators,
            peer_states_handle,
            mut events_receiver,
            ..
        } = handles;

        let peer_key = rng.gen::<node::SecretKey>().public();
        let mut fake_sync_state = test_validators.sync_state(1);
        fake_sync_state
            .last_contiguous_stored_block
            .message
            .proposal
            .number = BlockNumber(42);
        peer_states_handle.update(peer_key.clone(), fake_sync_state);
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::InvalidPeerUpdate(key) if key == peer_key);

        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert_matches!(events_receiver.try_recv(), None);
        Ok(())
    }
}

#[tokio::test]
async fn receiving_fake_sync_state_from_peer() {
    test_peer_states(PeerWithFakeSyncState).await;
}

#[derive(Debug)]
struct PeerWithFakeBlock;

#[async_trait]
impl Test for PeerWithFakeBlock {
    const BLOCK_COUNT: usize = 10;

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            clock,
            mut rng,
            test_validators,
            peer_states_handle,
            storage,
            mut message_receiver,
            mut events_receiver,
        } = handles;

        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states_handle.update(peer_key.clone(), test_validators.sync_state(1));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response,
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(1));

        let mut fake_block = test_validators.final_blocks[2].clone();
        fake_block.justification.message.proposal.number = BlockNumber(1);
        response.send(Ok(fake_block)).unwrap();

        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(
            peer_event,
            PeerStateEvent::GotInvalidBlock {
                block_number: BlockNumber(1),
                peer_key: key,
            } if key == peer_key
        );
        clock.advance(BLOCK_SLEEP_INTERVAL);

        // The invalid block must not be saved.
        assert_matches!(events_receiver.try_recv(), None);
        assert!(storage.block(ctx, BlockNumber(1)).await?.is_none());

        // Since we don't ban misbehaving peers, the node will send a request to the same peer again.
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient, number, ..
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(1));

        Ok(())
    }
}

#[tokio::test]
async fn receiving_fake_block_from_peer() {
    test_peer_states(PeerWithFakeBlock).await;
}
