//! Tests focused on interaction with multiple peers.

use super::*;
use crate::tests::wait_for_stored_block;

#[derive(Debug)]
struct RequestingBlocksFromTwoPeers;

#[async_trait]
impl Test for RequestingBlocksFromTwoPeers {
    const BLOCK_COUNT: usize = 5;

    fn tweak_config(&self, config: &mut Config) {
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config.max_concurrent_blocks = 5;
        config.max_concurrent_blocks_per_peer = 1;
        // ^ Necessary for blocks numbers in tests to be deterministic
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
        let first_peer = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&first_peer, test_validators.sync_state(2))
            .unwrap();

        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number: first_peer_block_number,
            response: first_peer_response,
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, first_peer);
        assert!(
            first_peer_block_number == BlockNumber(1) || first_peer_block_number == BlockNumber(2)
        );
        tracing::info!(%first_peer_block_number,"received requrest");

        let second_peer = rng.gen::<node::SecretKey>().public();
        peer_states
            .update(&second_peer, test_validators.sync_state(4))
            .unwrap();
        clock.advance(BLOCK_SLEEP_INTERVAL);

        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number: second_peer_block_number,
            response: second_peer_response,
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, second_peer);
        assert!(
            second_peer_block_number == BlockNumber(1)
                || second_peer_block_number == BlockNumber(2)
        );
        tracing::info!(%second_peer_block_number,"received requrest");

        test_validators.send_block(first_peer_block_number, first_peer_response);
        wait_for_event(
            ctx,
            &mut events_receiver,
            |ev| matches!(ev,PeerStateEvent::GotBlock(num) if num == first_peer_block_number),
        )
        .await
        .unwrap();
        // The node shouldn't send more requests to the first peer since it would be beyond
        // its known latest block number (2).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert_matches!(message_receiver.try_recv(), None);

        peer_states
            .update(&first_peer, test_validators.sync_state(4))
            .unwrap();
        clock.advance(BLOCK_SLEEP_INTERVAL);
        // Now the actor can get block #3 from the peer.

        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number: first_peer_block_number,
            response: first_peer_response,
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, first_peer);
        assert!(
            first_peer_block_number == BlockNumber(3) || first_peer_block_number == BlockNumber(4)
        );
        tracing::info!(%first_peer_block_number,"received requrest");

        test_validators.send_block(first_peer_block_number, first_peer_response);
        wait_for_event(
            ctx,
            &mut events_receiver,
            |ev| matches!(ev,PeerStateEvent::GotBlock(num) if num == first_peer_block_number),
        )
        .await
        .unwrap();
        clock.advance(BLOCK_SLEEP_INTERVAL);

        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number: first_peer_block_number,
            response: first_peer_response,
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, first_peer);
        assert!(
            first_peer_block_number == BlockNumber(3) || first_peer_block_number == BlockNumber(4)
        );
        tracing::info!(%first_peer_block_number,"received requrest");

        test_validators.send_block(second_peer_block_number, second_peer_response);
        wait_for_event(
            ctx,
            &mut events_receiver,
            |ev| matches!(ev,PeerStateEvent::GotBlock(num) if num == second_peer_block_number),
        )
        .await
        .unwrap();
        test_validators.send_block(first_peer_block_number, first_peer_response);
        wait_for_event(
            ctx,
            &mut events_receiver,
            |ev| matches!(ev,PeerStateEvent::GotBlock(num) if num == first_peer_block_number),
        )
        .await
        .unwrap();
        // No more blocks should be requested from peers.
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert_matches!(message_receiver.try_recv(), None);

        wait_for_stored_block(ctx, storage.as_ref(), BlockNumber(4)).await?;
        Ok(())
    }
}

#[tokio::test]
async fn requesting_blocks_from_two_peers() {
    test_peer_states(RequestingBlocksFromTwoPeers).await;
}

#[derive(Debug, Clone, Copy)]
struct PeerBehavior {
    /// The peer will go offline after this block.
    last_block: BlockNumber,
    /// The peer will stop responding after this block, but will still announce `SyncState` updates.
    /// Logically, should be `<= last_block`.
    last_block_to_return: BlockNumber,
}

impl Default for PeerBehavior {
    fn default() -> Self {
        Self {
            last_block: BlockNumber(u64::MAX),
            last_block_to_return: BlockNumber(u64::MAX),
        }
    }
}

#[derive(Debug, Clone)]
struct RequestingBlocksFromMultiplePeers {
    peer_behavior: Vec<PeerBehavior>,
    max_concurrent_blocks_per_peer: usize,
    respond_probability: f64,
}

impl RequestingBlocksFromMultiplePeers {
    fn new(peer_count: usize, max_concurrent_blocks_per_peer: usize) -> Self {
        Self {
            peer_behavior: vec![PeerBehavior::default(); peer_count],
            max_concurrent_blocks_per_peer,
            respond_probability: 0.0,
        }
    }

    fn create_peers(&self, rng: &mut impl Rng) -> HashMap<node::PublicKey, PeerBehavior> {
        let last_block_number = BlockNumber(Self::BLOCK_COUNT as u64 - 1);
        let peers = self.peer_behavior.iter().copied().map(|behavior| {
            let behavior = PeerBehavior {
                last_block: behavior.last_block.min(last_block_number),
                last_block_to_return: behavior.last_block_to_return.min(last_block_number),
            };
            let peer_key = rng.gen::<node::SecretKey>().public();
            (peer_key, behavior)
        });
        peers.collect()
    }
}

#[async_trait]
impl Test for RequestingBlocksFromMultiplePeers {
    const BLOCK_COUNT: usize = 20;

    fn tweak_config(&self, config: &mut Config) {
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config.max_concurrent_blocks_per_peer = self.max_concurrent_blocks_per_peer;
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
        let peers = &self.create_peers(rng);

        scope::run!(ctx, |ctx, s| async {
            // Announce peer states.
            for (peer_key, peer) in peers {
                let last_block = peer.last_block.0 as usize;
                peer_states.update(&peer_key, test_validators.sync_state(last_block)).unwrap();
            }

            s.spawn_bg(async {
                let mut responses_by_peer: HashMap<_, Vec<_>> = HashMap::new();
                let mut requested_blocks = HashSet::new();
                while requested_blocks.len() < Self::BLOCK_COUNT - 1 {
                    let Ok(message) = message_receiver.recv(ctx).await else {
                        return Ok(()); // Test is finished
                    };
                    let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                        recipient,
                        number,
                        response,
                    }) = message;

                    tracing::trace!("Block #{number} requested from {recipient:?}");
                    assert!(number <= peers[&recipient].last_block);

                    if peers[&recipient].last_block_to_return < number {
                        tracing::trace!("Dropping request for block #{number} to {recipient:?}");
                        continue;
                    }

                    assert!(
                        requested_blocks.insert(number),
                        "Block #{number} requested twice from a responsive peer"
                    );
                    let peer_responses = responses_by_peer.entry(recipient).or_default();
                    peer_responses.push((number, response));
                    assert!(peer_responses.len() <= self.max_concurrent_blocks_per_peer);
                    if peer_responses.len() == self.max_concurrent_blocks_per_peer {
                        // Peer is at capacity, respond to a random request in order to progress
                        let idx = rng.gen_range(0..peer_responses.len());
                        let (number, response) = peer_responses.remove(idx);
                        test_validators.send_block(number, response);
                    }

                    // Respond to some other random requests.
                    for peer_responses in responses_by_peer.values_mut() {
                        // Indexes are reversed in order to not be affected by removals.
                        for idx in (0..peer_responses.len()).rev() {
                            if !rng.gen_bool(self.respond_probability) {
                                continue;
                            }
                            let (number, response) = peer_responses.remove(idx);
                            test_validators.send_block(number, response);
                        }
                    }
                }

                // Answer to all remaining responses
                for (number, response) in responses_by_peer.into_values().flatten() {
                    test_validators.send_block(number, response);
                }
                Ok(())
            });

            // We advance the clock when a node receives a new block or updates a peer state,
            // since in both cases some new blocks may become available for download.
            let mut block_numbers = HashSet::with_capacity(Self::BLOCK_COUNT - 1);
            while block_numbers.len() < Self::BLOCK_COUNT - 1 {
                let peer_event = events_receiver.recv(ctx).await?;
                match peer_event {
                    PeerStateEvent::GotBlock(number) => {
                        assert!(
                            block_numbers.insert(number),
                            "Block #{number} received twice"
                        );
                        clock.advance(BLOCK_SLEEP_INTERVAL);
                    }
                    PeerStateEvent::RpcFailed{..} | PeerStateEvent::PeerDropped(_) => { /* Do nothing */ }
                    _ => panic!("Unexpected peer event: {peer_event:?}"),
                }
            }

            wait_for_stored_block(ctx, storage.as_ref(), BlockNumber(19)).await?;
            Ok(())
        })
        .await
    }
}

const RESPOND_PROBABILITIES: [f64; 5] = [0.0, 0.1, 0.2, 0.5, 0.9];

#[test_casing(15, Product(([1, 2, 3], RESPOND_PROBABILITIES)))]
#[tokio::test]
async fn requesting_blocks(max_concurrent_blocks_per_peer: usize, respond_probability: f64) {
    let mut test = RequestingBlocksFromMultiplePeers::new(3, max_concurrent_blocks_per_peer);
    test.respond_probability = respond_probability;
    test_peer_states(test.clone()).await;
}

#[test_casing(15, Product(([1, 2, 3], RESPOND_PROBABILITIES)))]
#[tokio::test]
async fn requesting_blocks_with_failures(
    max_concurrent_blocks_per_peer: usize,
    respond_probability: f64,
) {
    let mut test = RequestingBlocksFromMultiplePeers::new(3, max_concurrent_blocks_per_peer);
    test.respond_probability = respond_probability;
    test.peer_behavior[0].last_block = BlockNumber(5);
    test.peer_behavior[1].last_block = BlockNumber(15);
    test_peer_states(test).await;
}

#[test_casing(15, Product(([1, 2, 3], RESPOND_PROBABILITIES)))]
#[tokio::test]
async fn requesting_blocks_with_unreliable_peers(
    max_concurrent_blocks_per_peer: usize,
    respond_probability: f64,
) {
    let mut test = RequestingBlocksFromMultiplePeers::new(3, max_concurrent_blocks_per_peer);
    test.respond_probability = respond_probability;
    test.peer_behavior[0].last_block_to_return = BlockNumber(5);
    test.peer_behavior[1].last_block_to_return = BlockNumber(15);
    test_peer_states(test).await;
}
