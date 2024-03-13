use super::*;
use assert_matches::assert_matches;
use async_trait::async_trait;
use rand::{seq::IteratorRandom, Rng};
use std::{collections::HashSet, fmt};
use test_casing::{test_casing, Product};
use tracing::instrument;
use zksync_concurrency::{
    testonly::{abort_on_panic, set_timeout},
    time,
};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::testonly::new_store;

mod basics;
mod fakes;
mod multiple_peers;

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
    setup: validator::testonly::Setup,
    peer_states: Arc<PeerStates>,
    storage: Arc<BlockStore>,
    message_receiver: channel::UnboundedReceiver<io::OutputMessage>,
    events_receiver: channel::UnboundedReceiver<PeerStateEvent>,
}

#[async_trait]
trait Test: fmt::Debug + Send + Sync {
    const BLOCK_COUNT: usize;
    // TODO: move this to genesis
    const GENESIS_BLOCK_NUMBER: usize = 0;

    fn config(&self) -> Config {
        Config::new()
    }

    async fn initialize_storage(
        &self,
        _ctx: &ctx::Ctx,
        _storage: &BlockStore,
        _setup: &validator::testonly::Setup,
    ) {
        // Does nothing by default
    }

    async fn test(self, ctx: &ctx::Ctx, handles: TestHandles) -> anyhow::Result<()>;
}

#[instrument(level = "trace")]
async fn test_peer_states<T: Test>(test: T) {
    abort_on_panic();
    let _guard = set_timeout(TEST_TIMEOUT);

    let clock = ctx::ManualClock::new();
    let ctx = &ctx::test_root(&clock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new(rng, 4);
    setup.push_blocks(rng, T::BLOCK_COUNT);
    let (store, store_run) = new_store(ctx, &setup.genesis).await;
    test.initialize_storage(ctx, store.as_ref(), &setup).await;

    let (message_sender, message_receiver) = channel::unbounded();
    let (events_sender, events_receiver) = channel::unbounded();
    let mut peer_states = PeerStates::new(test.config(), store.clone(), message_sender);
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

#[tokio::test]
async fn test_try_acquire_peer_permit() {
    let clock = ctx::ManualClock::new();
    let ctx = &ctx::test_root(&clock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new(rng, 1);
    setup.push_blocks(rng, 10);
    scope::run!(ctx, |ctx, s| async {
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        let (send, _recv) = ctx::channel::unbounded();
        let peer_states = PeerStates::new(Config::default(), store, send);

        let peer: node::PublicKey = rng.gen();
        let b = &setup.blocks;
        for s in [
            // Empty entry.
            BlockStoreState {
                first: b[0].number(),
                last: None,
            },
            // Entry with some blocks.
            BlockStoreState {
                first: b[0].number(),
                last: Some(b[3].justification.clone()),
            },
            // Entry with changed first.
            BlockStoreState {
                first: b[1].number(),
                last: Some(b[3].justification.clone()),
            },
            // Empty entry again.
            BlockStoreState {
                first: b[1].number(),
                last: None,
            },
        ] {
            peer_states.update(&peer, s.clone()).unwrap();
            for block in b {
                let got = peer_states
                    .try_acquire_peer_permit(block.number())
                    .map(|p| p.0);
                if s.first <= block.number()
                    && s.last
                        .as_ref()
                        .map_or(false, |last| block.number() <= last.header().number)
                {
                    assert_eq!(Some(peer.clone()), got);
                } else {
                    assert_eq!(None, got);
                }
            }
        }
        Ok(())
    })
    .await
    .unwrap();
}
