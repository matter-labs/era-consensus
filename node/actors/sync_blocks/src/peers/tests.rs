use super::*;
use crate::tests::TestValidators;
use assert_matches::assert_matches;
use async_trait::async_trait;
use concurrency::time;
use rand::{rngs::StdRng, seq::IteratorRandom, Rng};
use std::{collections::HashSet, fmt};
use test_casing::{test_casing, Product};

const TEST_TIMEOUT: time::Duration = time::Duration::seconds(5);
const BLOCK_SLEEP_INTERVAL: time::Duration = time::Duration::milliseconds(5);

#[derive(Debug)]
struct TestHandles {
    clock: ctx::ManualClock,
    rng: StdRng,
    test_validators: TestValidators,
    peer_states_handle: PeerStatesHandle,
    storage: Arc<Storage>,
    message_receiver: channel::UnboundedReceiver<io::OutputMessage>,
    events_receiver: channel::UnboundedReceiver<PeerStateEvent>,
}

#[async_trait]
trait Test: fmt::Debug {
    const BLOCK_COUNT: usize;

    fn tweak_config(&self, _config: &mut Config) {
        // Does nothing by default
    }

    fn initialize_storage(&self, _storage: &Storage, _test_validators: &TestValidators) {
        // Does nothing by default
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()>;
}

#[instrument(level = "trace", skip(ctx, storage), err)]
async fn wait_for_stored_block(
    ctx: &ctx::Ctx,
    storage: &Storage,
    expected_block_number: BlockNumber,
) -> ctx::OrCanceled<()> {
    tracing::trace!("Started waiting for stored block");
    let mut subscriber = storage.subscribe_to_block_writes();
    let mut got_block = storage.get_last_contiguous_block_number();

    while got_block < expected_block_number {
        sync::changed(ctx, &mut subscriber).await?;
        got_block = storage.get_last_contiguous_block_number()
    }
    Ok(())
}

#[instrument(level = "trace")]
async fn test_peer_states<T: Test>(test: T) {
    concurrency::testonly::abort_on_panic();

    let storage_dir = tempfile::tempdir().unwrap();

    let ctx = &ctx::test_root(&ctx::RealClock).with_timeout(TEST_TIMEOUT);
    let clock = ctx::ManualClock::new();
    let ctx = &ctx::test_with_clock(ctx, &clock);
    let mut rng = ctx.rng();
    let test_validators = TestValidators::new(4, T::BLOCK_COUNT, &mut rng);
    let storage = Arc::new(Storage::new(
        &test_validators.final_blocks[0],
        storage_dir.path(),
    ));
    test.initialize_storage(&storage, &test_validators);

    let (message_sender, message_receiver) = channel::unbounded();
    let (events_sender, events_receiver) = channel::unbounded();
    let mut config = test_validators.test_config();
    test.tweak_config(&mut config);
    let (mut peer_states, peer_states_handle) =
        PeerStates::new(message_sender, storage.clone(), config);
    peer_states.events_sender = Some(events_sender);
    let test_handles = TestHandles {
        clock,
        rng,
        test_validators,
        peer_states_handle,
        storage,
        message_receiver,
        events_receiver,
    };

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(async {
            peer_states.run(ctx).await.or_else(|err| {
                if err.is::<ctx::Canceled>() {
                    Ok(()) // Swallow cancellation errors after the test is finished
                } else {
                    Err(err)
                }
            })
        });
        test.test(ctx, test_handles).await
    })
    .await
    .unwrap();
}

#[derive(Debug)]
struct UpdatingPeerStateWithSingleBlock;

#[async_trait]
impl Test for UpdatingPeerStateWithSingleBlock {
    const BLOCK_COUNT: usize = 2;

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            mut rng,
            test_validators,
            peer_states_handle,
            storage,
            mut message_receiver,
            mut events_receiver,
            ..
        } = handles;
        let mut storage_subscriber = storage.subscribe_to_block_writes();

        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states_handle.update(peer_key.clone(), test_validators.sync_state(1));
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
        assert_eq!(number, BlockNumber(1));

        // Emulate the peer sending a correct response.
        test_validators.send_block(BlockNumber(1), response);

        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::GotBlock(BlockNumber(1)));

        // Check that the block has been saved locally.
        let saved_block = *sync::changed(ctx, &mut storage_subscriber).await?;
        assert_eq!(saved_block, BlockNumber(1));
        Ok(())
    }
}

#[tokio::test]
async fn updating_peer_state_with_single_block() {
    test_peer_states(UpdatingPeerStateWithSingleBlock).await;
}

#[derive(Debug)]
struct UpdatingPeerStateWithMultipleBlocks;

impl UpdatingPeerStateWithMultipleBlocks {
    const MAX_CONCURRENT_BLOCKS: usize = 3;
}

#[async_trait]
impl Test for UpdatingPeerStateWithMultipleBlocks {
    const BLOCK_COUNT: usize = 10;

    fn tweak_config(&self, config: &mut Config) {
        config.max_concurrent_blocks_per_peer = Self::MAX_CONCURRENT_BLOCKS;
        // ^ We want to test rate limiting for peers
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
    }

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
        peer_states_handle.update(
            peer_key.clone(),
            test_validators.sync_state(Self::BLOCK_COUNT - 1),
        );
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);

        let mut requested_blocks = HashMap::with_capacity(Self::MAX_CONCURRENT_BLOCKS);
        for _ in 1..Self::BLOCK_COUNT {
            let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                recipient,
                number,
                response,
            }) = message_receiver.recv(ctx).await?;

            tracing::trace!("Received request for block #{number}");
            assert_eq!(recipient, peer_key);
            assert!(
                requested_blocks.insert(number, response).is_none(),
                "Block #{number} requested twice"
            );

            if requested_blocks.len() == Self::MAX_CONCURRENT_BLOCKS || rng.gen() {
                // Answer a random request.
                let number = *requested_blocks.keys().choose(&mut rng).unwrap();
                let response = requested_blocks.remove(&number).unwrap();
                test_validators.send_block(number, response);

                let peer_event = events_receiver.recv(ctx).await?;
                assert_matches!(peer_event, PeerStateEvent::GotBlock(got) if got == number);
            }
            clock.advance(BLOCK_SLEEP_INTERVAL);
        }

        // Answer all remaining requests.
        for (number, response) in requested_blocks {
            test_validators.send_block(number, response);
            let peer_event = events_receiver.recv(ctx).await?;
            assert_matches!(peer_event, PeerStateEvent::GotBlock(got) if got == number);
        }

        let expected_block_number = BlockNumber(Self::BLOCK_COUNT as u64 - 1);
        wait_for_stored_block(ctx, &storage, expected_block_number).await?;
        Ok(())
    }
}

#[tokio::test]
async fn updating_peer_state_with_multiple_blocks() {
    test_peer_states(UpdatingPeerStateWithMultipleBlocks).await;
}

#[derive(Debug)]
struct DownloadingBlocksInGaps {
    local_block_numbers: Vec<usize>,
    increase_peer_block_number_during_test: bool,
}

impl DownloadingBlocksInGaps {
    fn new(local_block_numbers: &[usize]) -> Self {
        Self {
            local_block_numbers: local_block_numbers
                .iter()
                .copied()
                .inspect(|&number| assert!(number > 0 && number < Self::BLOCK_COUNT))
                .collect(),
            increase_peer_block_number_during_test: false,
        }
    }
}

#[async_trait]
impl Test for DownloadingBlocksInGaps {
    const BLOCK_COUNT: usize = 10;

    fn tweak_config(&self, config: &mut Config) {
        config.max_concurrent_blocks = 1;
        // ^ Forces the node to download blocks in a deterministic order
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
    }

    fn initialize_storage(&self, storage: &Storage, test_validators: &TestValidators) {
        for &block_number in &self.local_block_numbers {
            storage.put_block(&test_validators.final_blocks[block_number]);
        }
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            clock,
            mut rng,
            test_validators,
            peer_states_handle,
            storage,
            mut message_receiver,
            ..
        } = handles;

        let peer_key = rng.gen::<node::SecretKey>().public();
        let mut last_peer_block_number = if self.increase_peer_block_number_during_test {
            rng.gen_range(1..Self::BLOCK_COUNT)
        } else {
            Self::BLOCK_COUNT - 1
        };
        peer_states_handle.update(
            peer_key.clone(),
            test_validators.sync_state(last_peer_block_number),
        );

        let expected_block_numbers =
            (1..Self::BLOCK_COUNT).filter(|number| !self.local_block_numbers.contains(number));

        // Check that all missing blocks are requested.
        for expected_number in expected_block_numbers {
            if expected_number > last_peer_block_number {
                last_peer_block_number = rng.gen_range(expected_number..Self::BLOCK_COUNT);
                peer_states_handle.update(
                    peer_key.clone(),
                    test_validators.sync_state(last_peer_block_number),
                );
                clock.advance(BLOCK_SLEEP_INTERVAL);
            }

            let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
                recipient,
                number,
                response,
            }) = message_receiver.recv(ctx).await?;

            assert_eq!(recipient, peer_key);
            assert_eq!(number.0 as usize, expected_number);
            test_validators.send_block(number, response);
            wait_for_stored_block(ctx, &storage, number).await?;
            clock.advance(BLOCK_SLEEP_INTERVAL);
        }
        Ok(())
    }
}

const LOCAL_BLOCK_NUMBERS: [&[usize]; 3] = [&[1, 9], &[3, 5, 6, 8], &[4]];

#[test_casing(6, Product((LOCAL_BLOCK_NUMBERS, [false, true])))]
#[tokio::test]
async fn downloading_blocks_in_gaps(
    local_block_numbers: &[usize],
    increase_peer_block_number_during_test: bool,
) {
    let mut test = DownloadingBlocksInGaps::new(local_block_numbers);
    test.increase_peer_block_number_during_test = increase_peer_block_number_during_test;
    test_peer_states(test).await;
}

#[derive(Debug)]
struct LimitingGetBlockConcurrency;

#[async_trait]
impl Test for LimitingGetBlockConcurrency {
    const BLOCK_COUNT: usize = 5;

    fn tweak_config(&self, config: &mut Config) {
        config.max_concurrent_blocks = 3;
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            mut rng,
            test_validators,
            peer_states_handle,
            storage,
            mut message_receiver,
            ..
        } = handles;
        let mut storage_subscriber = storage.subscribe_to_block_writes();

        let peer_key = rng.gen::<node::SecretKey>().public();
        peer_states_handle.update(
            peer_key.clone(),
            test_validators.sync_state(Self::BLOCK_COUNT - 1),
        );

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
        assert!(message_receiver.try_recv().is_none());
        assert_eq!(
            message_responses.keys().copied().collect::<HashSet<_>>(),
            HashSet::from([1, 2, 3])
        );

        // Send a correct response out of order.
        let response = message_responses.remove(&3).unwrap();
        test_validators.send_block(BlockNumber(3), response);

        let saved_block = *sync::changed(ctx, &mut storage_subscriber).await?;
        assert_eq!(saved_block, BlockNumber(3));

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

#[derive(Debug)]
struct RequestingBlocksFromTwoPeers;

#[async_trait]
impl Test for RequestingBlocksFromTwoPeers {
    const BLOCK_COUNT: usize = 5;

    fn tweak_config(&self, config: &mut Config) {
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
        config.max_concurrent_blocks = 2;
        config.max_concurrent_blocks_per_peer = 1;
        // ^ Necessary for blocks numbers in tests to be deterministic
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()> {
        let TestHandles {
            clock,
            mut rng,
            test_validators,
            peer_states_handle,
            storage,
            mut message_receiver,
            ..
        } = handles;

        let first_peer = rng.gen::<node::SecretKey>().public();
        peer_states_handle.update(first_peer.clone(), test_validators.sync_state(2));

        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response: first_peer_response,
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, first_peer);
        assert_eq!(number, BlockNumber(1));

        let second_peer = rng.gen::<node::SecretKey>().public();
        peer_states_handle.update(second_peer.clone(), test_validators.sync_state(4));
        clock.advance(BLOCK_SLEEP_INTERVAL);

        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response: second_peer_response,
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, second_peer);
        assert_eq!(number, BlockNumber(2));

        test_validators.send_block(BlockNumber(1), first_peer_response);
        // The node shouldn't send more requests to the first peer since it would be beyond
        // its known latest block number (2).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert!(message_receiver.try_recv().is_none());

        peer_states_handle.update(first_peer.clone(), test_validators.sync_state(4));
        clock.advance(BLOCK_SLEEP_INTERVAL);
        // Now the actor can get block #3 from the peer.

        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response: first_peer_response,
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, first_peer);
        assert_eq!(number, BlockNumber(3));

        test_validators.send_block(BlockNumber(3), first_peer_response);
        clock.advance(BLOCK_SLEEP_INTERVAL);

        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response: first_peer_response,
        }) = message_receiver.recv(ctx).await?;
        assert_eq!(recipient, first_peer);
        assert_eq!(number, BlockNumber(4));

        test_validators.send_block(BlockNumber(2), second_peer_response);
        test_validators.send_block(BlockNumber(4), first_peer_response);
        // No more blocks should be requested from peers.
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert!(message_receiver.try_recv().is_none());

        wait_for_stored_block(ctx, &storage, BlockNumber(4)).await?;
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
            mut rng,
            test_validators,
            peer_states_handle,
            storage,
            mut message_receiver,
            mut events_receiver,
        } = handles;

        let peers = &self.create_peers(&mut rng);

        scope::run!(ctx, |ctx, s| async {
            // Announce peer states.
            for (peer_key, peer) in peers {
                let last_block = peer.last_block.0 as usize;
                peer_states_handle.update(peer_key.clone(), test_validators.sync_state(last_block));
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
                    PeerStateEvent::PeerUpdated(_) => {
                        clock.advance(BLOCK_SLEEP_INTERVAL);
                    }
                    PeerStateEvent::PeerDisconnected(_) => { /* Do nothing */ }
                }
            }

            wait_for_stored_block(ctx, &storage, BlockNumber(19)).await?;
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

#[derive(Debug)]
struct DisconnectingPeer;

#[async_trait]
impl Test for DisconnectingPeer {
    const BLOCK_COUNT: usize = 5;

    fn tweak_config(&self, config: &mut Config) {
        config.sleep_interval_for_get_block = BLOCK_SLEEP_INTERVAL;
    }

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

        // Drop the response sender emulating peer disconnect.
        message_receiver.recv(ctx).await?;

        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerDisconnected(key) if key == peer_key);

        // Check that no new requests are sent (there are no peers to send them to).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert!(message_receiver.try_recv().is_none());

        // Re-connect the peer with an updated state.
        peer_states_handle.update(peer_key.clone(), test_validators.sync_state(2));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);
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
        test_validators.send_block(BlockNumber(2), response);
        drop(responses);

        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::GotBlock(BlockNumber(2)));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerDisconnected(key) if key == peer_key);

        // Check that no new requests are sent (there are no peers to send them to).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert!(message_receiver.try_recv().is_none());

        // Re-connect the peer with the same state.
        peer_states_handle.update(peer_key.clone(), test_validators.sync_state(2));
        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::PeerUpdated(key) if key == peer_key);
        clock.advance(BLOCK_SLEEP_INTERVAL);

        let message = message_receiver.recv(ctx).await?;
        let io::OutputMessage::Network(SyncBlocksInputMessage::GetBlock {
            recipient,
            number,
            response,
        }) = message;
        assert_eq!(recipient, peer_key);
        assert_eq!(number, BlockNumber(1));
        test_validators.send_block(BlockNumber(1), response);

        let peer_event = events_receiver.recv(ctx).await?;
        assert_matches!(peer_event, PeerStateEvent::GotBlock(BlockNumber(1)));

        // Check that no new requests are sent (all blocks are downloaded).
        clock.advance(BLOCK_SLEEP_INTERVAL);
        assert!(message_receiver.try_recv().is_none());

        wait_for_stored_block(ctx, &storage, BlockNumber(2)).await?;
        Ok(())
    }
}

#[tokio::test]
async fn disconnecting_peer() {
    test_peer_states(DisconnectingPeer).await;
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

#[test]
fn processing_invalid_sync_states() {
    let storage_dir = tempfile::tempdir().unwrap();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let test_validators = TestValidators::new(4, 3, rng);
    let storage = Arc::new(Storage::new(
        &test_validators.final_blocks[0],
        storage_dir.path(),
    ));

    let (message_sender, _) = channel::unbounded();
    let (peer_states, _) = PeerStates::new(message_sender, storage, test_validators.test_config());

    let mut invalid_sync_state = test_validators.sync_state(1);
    invalid_sync_state.first_stored_block = test_validators.final_blocks[2].justification.clone();
    let err = peer_states
        .validate_sync_state(invalid_sync_state)
        .unwrap_err();
    let err = format!("{err:?}");
    assert!(err.contains("first_stored_block"), "{err}");

    let mut invalid_sync_state = test_validators.sync_state(1);
    invalid_sync_state.last_contiguous_stored_block =
        test_validators.final_blocks[2].justification.clone();
    let err = peer_states
        .validate_sync_state(invalid_sync_state)
        .unwrap_err();
    let err = format!("{err:?}");
    assert!(err.contains("last_contiguous_stored_block"), "{err}");

    let mut invalid_sync_state = test_validators.sync_state(1);
    invalid_sync_state
        .last_contiguous_stored_block
        .message
        .proposal_block_number = BlockNumber(5);
    invalid_sync_state
        .last_stored_block
        .message
        .proposal_block_number = BlockNumber(5);
    let err = peer_states
        .validate_sync_state(invalid_sync_state)
        .unwrap_err();
    let err = format!("{err:?}");
    assert!(
        err.contains("Failed verifying `last_contiguous_stored_block`"),
        "{err}"
    );

    let other_network = TestValidators::new(4, 2, rng);
    let invalid_sync_state = other_network.sync_state(1);
    let err = peer_states
        .validate_sync_state(invalid_sync_state)
        .unwrap_err();
    let err = format!("{err:?}");
    assert!(
        err.contains("Failed verifying `last_contiguous_stored_block`"),
        "{err}"
    );
}

#[test]
fn processing_invalid_blocks() {
    let storage_dir = tempfile::tempdir().unwrap();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let test_validators = TestValidators::new(4, 3, rng);
    let storage = Arc::new(Storage::new(
        &test_validators.final_blocks[0],
        storage_dir.path(),
    ));

    let (message_sender, _) = channel::unbounded();
    let (peer_states, _) = PeerStates::new(message_sender, storage, test_validators.test_config());

    let invalid_block = &test_validators.final_blocks[0];
    let err = peer_states
        .validate_block(BlockNumber(1), invalid_block)
        .unwrap_err();
    let err = format!("{err:?}");
    assert!(err.contains("does not have requested number"), "{err}");

    let mut invalid_block = test_validators.final_blocks[1].clone();
    invalid_block.justification = test_validators.final_blocks[0].justification.clone();
    let err = peer_states
        .validate_block(BlockNumber(1), &invalid_block)
        .unwrap_err();
    let err = format!("{err:?}");
    assert!(
        err.contains("numbers in `block` and quorum certificate"),
        "{err}"
    );

    let mut invalid_block = test_validators.final_blocks[1].clone();
    invalid_block.block.payload = b"invalid".to_vec();
    let err = peer_states
        .validate_block(BlockNumber(1), &invalid_block)
        .unwrap_err();
    let err = format!("{err:?}");
    assert!(
        err.contains("hashes in `block` and quorum certificate"),
        "{err}"
    );

    let other_network = TestValidators::new(4, 2, rng);
    let invalid_block = &other_network.final_blocks[1];
    let err = peer_states
        .validate_block(BlockNumber(1), invalid_block)
        .unwrap_err();
    let err = format!("{err:?}");
    assert!(err.contains("verifying quorum certificate"), "{err}");
}
