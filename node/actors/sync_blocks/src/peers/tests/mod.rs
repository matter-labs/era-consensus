use super::*;
use crate::tests::make_store;
use crate::tests::TestValidators;
use assert_matches::assert_matches;
use async_trait::async_trait;
use rand::{rngs::StdRng, seq::IteratorRandom, Rng};
use std::{collections::HashSet, fmt};
use test_casing::{test_casing, Product};
use zksync_concurrency::{testonly::abort_on_panic, time};
use tracing::instrument;

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
    peer_states: Arc<PeerStates>,
    storage: Arc<BlockStore>,
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
        _storage: &BlockStore,
        _test_validators: &TestValidators,
    ) {
        // Does nothing by default
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()>;
}

#[instrument(level = "trace")]
async fn test_peer_states<T: Test>(test: T) {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::RealClock).with_timeout(TEST_TIMEOUT);
    let clock = ctx::ManualClock::new();
    let ctx = &ctx::test_with_clock(ctx, &clock);
    let mut rng = ctx.rng();
    let test_validators = TestValidators::new(4, T::BLOCK_COUNT, &mut rng);
    let storage = make_store(ctx,test_validators.final_blocks[T::GENESIS_BLOCK_NUMBER].clone()).await;
    test.initialize_storage(ctx, storage.as_ref(), &test_validators)
        .await;

    let (message_sender, message_receiver) = channel::unbounded();
    let (events_sender, events_receiver) = channel::unbounded();
    let mut config = test_validators.test_config();
    test.tweak_config(&mut config);
    let mut peer_states = PeerStates::new(config, storage.clone(), message_sender);
    peer_states.events_sender = Some(events_sender);
    let peer_states = Arc::new(peer_states);
    let test_handles = TestHandles {
        clock,
        rng,
        test_validators,
        peer_states: peer_states.clone(),
        storage,
        message_receiver,
        events_receiver,
    };

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(async {
            peer_states.run_block_fetcher(ctx).await.ok();
            Ok(())
        });
        test.test(ctx, test_handles).await
    })
    .await
    .unwrap();
}
