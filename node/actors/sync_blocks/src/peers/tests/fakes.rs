//! Tests focused on handling peers providing fake information to the node.

use super::*;
use crate::tests::sync_state;
use zksync_consensus_roles::{validator, validator::testonly::Setup};
use zksync_consensus_storage::testonly::new_store;

#[tokio::test]
async fn processing_invalid_sync_states() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::builder(rng, 4)
        .push_blocks(rng, 3)
        .build();
    let (storage, _runner) = new_store(ctx, &setup.genesis).await;

    let (message_sender, _) = channel::unbounded();
    let peer_states = PeerStates::new(Config::new(setup.genesis.clone()), storage, message_sender);
    let peer = &rng.gen::<node::SecretKey>().public();
    
    let mut invalid_block = setup.blocks[1].clone();
    invalid_block.justification.message.proposal.number = rng.gen();
    let invalid_sync_state = sync_state(&setup, Some(&invalid_block));
    assert!(peer_states.update(peer, invalid_sync_state).is_err());

    let other_network = Setup::builder(rng, 4)
        .push_blocks(rng, 2)
        .build();
    let invalid_sync_state = sync_state(&other_network, other_network.blocks.get(1));
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
            setup,
            peer_states,
            mut events_receiver,
            ..
        } = handles;

        let rng = &mut ctx.rng();
        let peer_key = rng.gen::<node::SecretKey>().public();
        let mut invalid_block = setup.blocks[1].clone();
        invalid_block.justification.message.proposal.number = rng.gen();
        let fake_sync_state = sync_state(&setup, Some(&invalid_block));
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

    fn config(&self, setup: &Setup) -> Config {
        let mut cfg = Config::new(setup.genesis.clone());
        cfg.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        cfg
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

        for fake_block in [
            // other block than requested
            setup.blocks[1].clone(),
            // block with wrong validator set
            Setup::builder(rng, 4).push_blocks(rng, 1).build().blocks[0].clone(),
            // block with mismatching payload,
            {
                let mut block = setup.blocks[0].clone();
                block.payload = validator::Payload(b"invalid".to_vec());
                block
            },
        ] {
            let key = rng.gen::<node::SecretKey>().public();
            peer_states
                .update(&key, sync_state(&setup, setup.blocks.get(0)))
                .unwrap();
            clock.advance(BLOCK_SLEEP_INTERVAL);

            let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                recipient,
                number,
                response,
            }) = message_receiver.recv(ctx).await?;
            assert_eq!(recipient, key);
            assert_eq!(number, setup.blocks[0].number());
            response.send(Ok(fake_block)).unwrap();

            wait_for_event(ctx, &mut events_receiver, |ev| {
                matches!(ev,
                    PeerStateEvent::RpcFailed {
                        block_number,
                        peer_key,
                    } if peer_key == key && block_number == number
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
