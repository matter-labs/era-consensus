use super::*;
use crate::tests::TestValidators;
use assert_matches::assert_matches;
use async_trait::async_trait;
use rand::{rngs::StdRng, seq::IteratorRandom, Rng};
use std::{collections::HashSet, fmt};
use test_casing::{test_casing, Product};
use zksync_concurrency::{testonly::abort_on_panic, time};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::InMemoryStorage;

mod basics;
mod fakes;
mod multiple_peers;
mod snapshots;

const TEST_TIMEOUT: time::Duration = time::Duration::seconds(5);
const BLOCK_SLEEP_INTERVAL: time::Duration = time::Duration::milliseconds(5);

#[derive(Debug)]
struct TestHandles {
    clock: ctx::ManualClock,
    rng: StdRng,
    test_validators: TestValidators,
    peer_states_handle: PeerStatesHandle,
    storage: Arc<dyn WriteBlockStore>,
    message_receiver: channel::UnboundedReceiver<io::OutputMessage>,
    events_receiver: channel::UnboundedReceiver<PeerStateEvent>,
}

#[async_trait]
trait Test: fmt::Debug + Send + Sync {
    const BLOCK_COUNT: usize;
    const GENESIS_BLOCK_NUMBER: usize = 0;

    fn tweak_config(&self, _config: &mut Config) {
        // Does nothing by default
    }

    async fn initialize_storage(
        &self,
        _ctx: &ctx::Ctx,
        _storage: &dyn WriteBlockStore,
        _test_validators: &TestValidators,
    ) {
        // Does nothing by default
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()>;
}

#[instrument(level = "trace", skip(ctx, storage), err)]
async fn wait_for_stored_block(
    ctx: &ctx::Ctx,
    storage: &dyn WriteBlockStore,
    expected_block_number: BlockNumber,
) -> ctx::OrCanceled<()> {
    tracing::trace!("Started waiting for stored block");
    let mut subscriber = storage.subscribe_to_block_writes();
    let mut got_block = storage.last_contiguous_block_number(ctx).await.unwrap();

    while got_block < expected_block_number {
        sync::changed(ctx, &mut subscriber).await?;
        got_block = storage.last_contiguous_block_number(ctx).await.unwrap();
    }
    Ok(())
}

#[instrument(level = "trace", skip(ctx, events_receiver))]
async fn wait_for_peer_update(
    ctx: &ctx::Ctx,
    events_receiver: &mut channel::UnboundedReceiver<PeerStateEvent>,
    expected_peer: &node::PublicKey,
) -> ctx::OrCanceled<()> {
    loop {
        let peer_event = events_receiver.recv(ctx).await?;
        tracing::trace!(?peer_event, "received peer event");
        match peer_event {
            PeerStateEvent::PeerUpdated(key) => {
                assert_eq!(key, *expected_peer);
                return Ok(());
            }
            PeerStateEvent::PeerDisconnected(_) | PeerStateEvent::GotBlock(_) => {
                // Skip update
            }
            _ => panic!("Received unexpected peer event: {peer_event:?}"),
        }
    }
}

#[instrument(level = "trace")]
async fn test_peer_states<T: Test>(test: T) {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::RealClock).with_timeout(TEST_TIMEOUT);
    let clock = ctx::ManualClock::new();
    let ctx = &ctx::test_with_clock(ctx, &clock);
    let mut rng = ctx.rng();
    let test_validators = TestValidators::new(4, T::BLOCK_COUNT, &mut rng);
    let storage =
        InMemoryStorage::new(test_validators.final_blocks[T::GENESIS_BLOCK_NUMBER].clone());
    let storage = Arc::new(storage);
    test.initialize_storage(ctx, storage.as_ref(), &test_validators)
        .await;

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
            peer_states.run(ctx).await.or_else(|err| match err {
                ctx::Error::Canceled(_) => Ok(()), // Swallow cancellation errors after the test is finished
                ctx::Error::Internal(err) => Err(err),
            })
        });
        test.test(ctx, test_handles).await
    })
    .await
    .unwrap();
}
