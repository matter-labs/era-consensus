use super::*;
use assert_matches::assert_matches;
use async_trait::async_trait;
use rand::{seq::IteratorRandom, Rng};
use std::{collections::HashSet, fmt};
use test_casing::{test_casing, Product};
use tracing::instrument;
use zksync_consensus_roles::validator;
use zksync_consensus_network as network;
use zksync_concurrency::{testonly::abort_on_panic, time};
use crate::tests::test_config;

mod basics;
mod fakes;
mod multiple_peers;
mod snapshots;

const TEST_TIMEOUT: time::Duration = time::Duration::seconds(5);
const BLOCK_SLEEP_INTERVAL: time::Duration = time::Duration::milliseconds(5);

async fn wait_for_event(
    ctx: &ctx::Ctx,
    events: &mut channel::UnboundedReceiver<PeerStateEvent>,
    pred: impl Fn(PeerStateEvent) -> bool,
) -> ctx::OrCanceled<()> {
    while !pred(events.recv(ctx).await?) {}
    Ok(())
}

#[derive(Debug)]
struct TestHandles {
    clock: ctx::ManualClock,
    setup: validator::testonly::GenesisSetup,
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
        _setup: &validator::testonly::GenesisSetup,
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
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::GenesisSetup::new(rng,4);
    setup.push_blocks(rng, T::BLOCK_COUNT);
    let (store, store_run) = network::testonly::new_store(ctx,&setup.blocks[T::GENESIS_BLOCK_NUMBER]).await;
    test.initialize_storage(ctx, store.as_ref(), &setup).await;

    let (message_sender, message_receiver) = channel::unbounded();
    let (events_sender, events_receiver) = channel::unbounded();
    let mut config = test_config(&setup);
    test.tweak_config(&mut config);
    let mut peer_states = PeerStates::new(config, store.clone(), message_sender);
    peer_states.events_sender = Some(events_sender);
    let peer_states = Arc::new(peer_states);
    let test_handles = TestHandles {
        clock,
        setup,
        peer_states: peer_states.clone(),
        storage: store.clone(),
        message_receiver,
        events_receiver,
    };

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(store_run.run(ctx));
        s.spawn_bg(async {
            peer_states.run_block_fetcher(ctx).await.ok();
            Ok(())
        });
        test.test(ctx, test_handles).await
    })
    .await
    .unwrap();
}
