//! Tests focused on handling peers providing fake information to the node.

use super::*;
use zksync_consensus_roles::validator;

#[tokio::test]
async fn processing_invalid_sync_states() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let test_validators = TestValidators::new(rng, 4, 3);
    let (storage, _runner) = make_store(ctx, test_validators.final_blocks[0].clone()).await;

    let (message_sender, _) = channel::unbounded();
    let peer_states = PeerStates::new(test_validators.test_config(), storage, message_sender);

    let peer = &rng.gen::<node::SecretKey>().public();
    let mut invalid_sync_state = test_validators.sync_state(1);
    invalid_sync_state.first = test_validators.final_blocks[2].justification.clone();
    assert!(peer_states.update(peer, invalid_sync_state).is_err());

    let mut invalid_sync_state = test_validators.sync_state(1);
    invalid_sync_state.last.message.proposal.number = BlockNumber(5);
    assert!(peer_states.update(peer, invalid_sync_state).is_err());

    let other_network = TestValidators::new(rng, 4, 2);
    let invalid_sync_state = other_network.sync_state(1);
    assert!(peer_states.update(peer, invalid_sync_state).is_err());
}

#[derive(Debug)]
struct PeerWithFakeSyncState;

#[async_trait]
impl Test for PeerWithFakeSyncState {
    const BLOCK_COUNT: usize = 10;

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            clock,
            test_validators,
            peer_states,
            mut events_receiver,
            ..
        } = handles;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        let mut fake_sync_state = test_validators.sync_state(1);
        fake_sync_state.last.message.proposal.number = BlockNumber(42);
        assert!(peer_states.update(&peer_key, fake_sync_state).is_err());

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

    fn tweak_config(&self, cfg: &mut Config) {
        cfg.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            clock,
            test_validators,
            peer_states,
            storage,
            mut message_receiver,
            mut events_receiver,
        } = handles;

        let rng = &mut ctx.rng();

        for fake_block in [
            // other block than requested
            test_validators.final_blocks[0].clone(),
            // block with wrong validator set
            TestValidators::new(rng, 4, 2).final_blocks[1].clone(),
            // block with mismatching payload,
            {
                let mut block = test_validators.final_blocks[1].clone();
                block.payload = validator::Payload(b"invalid".to_vec());
                block
            },
        ] {
            let peer_key = rng.gen::<node::SecretKey>().public();
            peer_states
                .update(&peer_key, test_validators.sync_state(1))
                .unwrap();
            clock.advance(BLOCK_SLEEP_INTERVAL);

            let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                recipient,
                number,
                response,
            }) = message_receiver.recv(ctx).await?;
            assert_eq!(recipient, peer_key);
            assert_eq!(number, BlockNumber(1));
            response.send(Ok(fake_block)).unwrap();

            wait_for_event(ctx, &mut events_receiver, |ev| {
                matches!(ev,
                    PeerStateEvent::RpcFailed {
                        block_number: BlockNumber(1),
                        peer_key: key,
                    } if key == peer_key
                )
            })
            .await?;
        }

        // The invalid block must not be saved.
        assert!(storage.block(ctx, BlockNumber(1)).await?.is_none());
        Ok(())
    }
}

#[tokio::test]
async fn receiving_fake_block_from_peer() {
    test_peer_states(PeerWithFakeBlock).await;
}
