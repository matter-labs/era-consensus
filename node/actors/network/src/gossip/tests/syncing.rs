//! Integration tests of block synchronization.
use crate::testonly;
use anyhow::Context as _;
use rand::seq::SliceRandom as _;
use test_casing::{test_casing, Product};
use tracing::Instrument as _;
use zksync_concurrency::{
    ctx, limiter, scope,
    testonly::{abort_on_panic, set_timeout},
    time,
};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::{
    testonly::{dump, in_memory, TestMemoryStorage},
    BlockStore,
};

const EXCHANGED_STATE_COUNT: usize = 5;
const NETWORK_CONNECTIVITY_CASES: [(usize, usize); 5] = [(2, 1), (3, 2), (5, 3), (10, 4), (10, 7)];

/// Tests block syncing with global network synchronization (a next block becoming available
/// on some node only after nodes have received the previous block.
#[test_casing(5, NETWORK_CONNECTIVITY_CASES)]
#[tokio::test(flavor = "multi_thread")]
async fn coordinated_block_syncing(node_count: usize, gossip_peers: usize) {
    abort_on_panic();
    let _guard = set_timeout(time::Duration::seconds(20));

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new(rng, node_count);
    setup.push_blocks(rng, EXCHANGED_STATE_COUNT);
    let cfgs = testonly::new_configs(rng, &setup, gossip_peers);
    scope::run!(ctx, |ctx, s| async {
        let mut nodes = vec![];
        for (i, mut cfg) in cfgs.into_iter().enumerate() {
            cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
            cfg.rpc.get_block_rate = limiter::Rate::INF;
            cfg.rpc.get_block_timeout = None;
            cfg.validator_key = None;
            let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
            s.spawn_bg(store.runner.run(ctx));
            let (node, runner) = testonly::Instance::new(cfg, store.blocks);
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            nodes.push(node);
        }
        for block in &setup.blocks {
            nodes
                .choose(rng)
                .unwrap()
                .net
                .gossip
                .block_store
                .queue_block(ctx, block.clone().into())
                .await
                .context("queue_block()")?;
            for node in &nodes {
                node.net
                    .gossip
                    .block_store
                    .wait_until_persisted(ctx, block.number())
                    .await
                    .unwrap();
            }
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Tests block syncing in an uncoordinated network, in which new blocks arrive at a schedule.
#[test_casing(10, Product((
    NETWORK_CONNECTIVITY_CASES,
    [time::Duration::milliseconds(50), time::Duration::milliseconds(500)],
)))]
#[tokio::test(flavor = "multi_thread")]
async fn uncoordinated_block_syncing(
    (node_count, gossip_peers): (usize, usize),
    state_generation_interval: time::Duration,
) {
    abort_on_panic();
    let _guard = set_timeout(time::Duration::seconds(20));

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new(rng, node_count);
    setup.push_blocks(rng, EXCHANGED_STATE_COUNT);
    let cfgs = testonly::new_configs(rng, &setup, gossip_peers);
    scope::run!(ctx, |ctx, s| async {
        let mut nodes = vec![];
        for (i, mut cfg) in cfgs.into_iter().enumerate() {
            cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
            cfg.rpc.get_block_rate = limiter::Rate::INF;
            cfg.rpc.get_block_timeout = None;
            cfg.validator_key = None;
            let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
            s.spawn_bg(store.runner.clone().run(ctx));
            let (node, runner) = testonly::Instance::new(cfg, store.blocks);
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            nodes.push(node);
        }
        for block in &setup.blocks {
            nodes
                .choose(rng)
                .unwrap()
                .net
                .gossip
                .block_store
                .queue_block(ctx, block.clone())
                .await
                .context("queue_block()")?;
            ctx.sleep(state_generation_interval).await?;
        }
        let last = setup.blocks.last().unwrap().number();
        for node in &nodes {
            node.net
                .gossip
                .block_store
                .wait_until_persisted(ctx, last)
                .await
                .unwrap();
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Test concurrently adding new nodes and new blocks to the network.
#[tokio::test(flavor = "multi_thread")]
async fn test_switching_on_nodes() {
    abort_on_panic();
    let _guard = set_timeout(time::Duration::seconds(20));

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new(rng, 7);
    // It is important that all nodes will connect to each other,
    // because we spawn the nodes gradually and we want the network
    // to be connected at all times.
    let cfgs = testonly::new_configs(rng, &setup, setup.validator_keys.len());
    setup.push_blocks(rng, cfgs.len());
    scope::run!(ctx, |ctx, s| async {
        let mut nodes = vec![];
        for (i, mut cfg) in cfgs.into_iter().enumerate() {
            // Spawn another node.
            cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
            cfg.rpc.get_block_rate = limiter::Rate::INF;
            cfg.rpc.get_block_timeout = None;
            cfg.validator_key = None;
            let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
            s.spawn_bg(store.runner.run(ctx));
            let (node, runner) = testonly::Instance::new(cfg, store.blocks);
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            nodes.push(node);

            // Insert a block to storage of a random node.
            nodes
                .choose(rng)
                .unwrap()
                .net
                .gossip
                .block_store
                .queue_block(ctx, setup.blocks[i].clone())
                .await
                .context("queue_block()")?;

            // Wait for all the nodes to fetch the block.
            for node in &nodes {
                node.net
                    .gossip
                    .block_store
                    .wait_until_persisted(ctx, setup.blocks[i].number())
                    .await
                    .unwrap();
            }
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Test concurrently removing nodes and adding new blocks to the network.
#[tokio::test(flavor = "multi_thread")]
async fn test_switching_off_nodes() {
    abort_on_panic();
    let _guard = set_timeout(time::Duration::seconds(20));

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new(rng, 7);
    // It is important that all nodes will connect to each other,
    // because we spawn the nodes gradually and we want the network
    // to be connected at all times.
    let cfgs = testonly::new_configs(rng, &setup, setup.validator_keys.len());
    setup.push_blocks(rng, cfgs.len());
    scope::run!(ctx, |ctx, s| async {
        let mut nodes = vec![];
        for (i, mut cfg) in cfgs.into_iter().enumerate() {
            // Spawn another node.
            cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
            cfg.rpc.get_block_rate = limiter::Rate::INF;
            cfg.rpc.get_block_timeout = None;
            cfg.validator_key = None;
            let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
            s.spawn_bg(store.runner.run(ctx));
            let (node, runner) = testonly::Instance::new(cfg, store.blocks);
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            nodes.push(node);
        }
        nodes.shuffle(rng);

        for i in 0..nodes.len() {
            // Insert a block to storage of a random node.
            nodes[i..]
                .choose(rng)
                .unwrap()
                .net
                .gossip
                .block_store
                .queue_block(ctx, setup.blocks[i].clone())
                .await
                .context("queue_block()")?;

            // Wait for all the remaining nodes to fetch the block.
            for node in &nodes[i..] {
                node.net
                    .gossip
                    .block_store
                    .wait_until_persisted(ctx, setup.blocks[i].number())
                    .await
                    .unwrap();
            }

            // Terminate a random node.
            nodes[i].terminate(ctx).await.unwrap();
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Test checking that nodes with different first block can synchronize.
#[tokio::test(flavor = "multi_thread")]
async fn test_different_first_block() {
    abort_on_panic();
    let _guard = set_timeout(time::Duration::seconds(20));

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new(rng, 4);
    setup.push_blocks(rng, 10);
    // It is important that all nodes will connect to each other,
    // because we spawn the nodes gradually and we want the network
    // to be connected at all times.
    let cfgs = testonly::new_configs(rng, &setup, setup.validator_keys.len());
    scope::run!(ctx, |ctx, s| async {
        let mut nodes = vec![];
        for (i, mut cfg) in cfgs.into_iter().enumerate() {
            // Spawn another node.
            cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
            cfg.rpc.get_block_rate = limiter::Rate::INF;
            cfg.rpc.get_block_timeout = None;
            cfg.validator_key = None;
            // Choose the first block for the node at random.
            let first = setup.blocks.choose(rng).unwrap().number();
            let store =
                TestMemoryStorage::new_store_with_first_block(ctx, &setup.genesis, first).await;
            s.spawn_bg(store.runner.run(ctx));
            let (node, runner) = testonly::Instance::new(cfg, store.blocks);
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            nodes.push(node);
        }
        nodes.shuffle(rng);

        for block in &setup.blocks {
            // Find nodes interested in the next block.
            let interested_nodes: Vec<_> = nodes
                .iter()
                .filter(|n| n.net.gossip.block_store.queued().first <= block.number())
                .collect();
            // Store this block to one of them.
            if let Some(node) = interested_nodes.choose(rng) {
                node.net
                    .gossip
                    .block_store
                    .queue_block(ctx, block.clone())
                    .await
                    .unwrap();
            }
            // Wait until all remaining nodes get the new block.
            for node in interested_nodes {
                node.net
                    .gossip
                    .block_store
                    .wait_until_persisted(ctx, block.number())
                    .await
                    .unwrap();
            }
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Test checking that if blocks that weren't queued get persisted,
/// the syncing can behave accordingly.
#[tokio::test(flavor = "multi_thread")]
async fn test_sidechannel_sync() {
    abort_on_panic();
    let _guard = set_timeout(time::Duration::seconds(20));

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new(rng, 2);
    setup.push_blocks(rng, 10);
    let cfgs = testonly::new_configs(rng, &setup, 1);
    scope::run!(ctx, |ctx, s| async {
        let mut stores = vec![];
        let mut nodes = vec![];
        for (i, mut cfg) in cfgs.into_iter().enumerate() {
            cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
            cfg.rpc.get_block_rate = limiter::Rate::INF;
            cfg.rpc.get_block_timeout = None;
            cfg.validator_key = None;

            // Build a custom persistent store, so that we can tweak it later.
            let persistent =
                in_memory::BlockStore::new(setup.genesis.clone(), setup.genesis.first_block);
            stores.push(persistent.clone());
            let (block_store, runner) = BlockStore::new(ctx, Box::new(persistent)).await?;
            s.spawn_bg(runner.run(ctx));
            // Use the standard batch store since it doesn't matter.
            let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
            let (node, runner) = testonly::Instance::new(cfg, block_store);
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            nodes.push(node);
        }

        {
            // Truncate at the start.
            stores[1].truncate(setup.blocks[3].number());

            // Sync a block prefix.
            let prefix = &setup.blocks[0..5];
            for b in prefix {
                nodes[0]
                    .net
                    .gossip
                    .block_store
                    .queue_block(ctx, b.clone())
                    .await?;
            }
            nodes[1]
                .net
                .gossip
                .block_store
                .wait_until_persisted(ctx, prefix.last().unwrap().number())
                .await?;

            // Check that the expected block range is actually stored.
            assert_eq!(setup.blocks[3..5], dump(ctx, &stores[1]).await);
        }

        {
            // Truncate more than prefix.
            stores[1].truncate(setup.blocks[8].number());

            // Sync a block suffix.
            let suffix = &setup.blocks[5..10];
            for b in suffix {
                nodes[0]
                    .net
                    .gossip
                    .block_store
                    .queue_block(ctx, b.clone())
                    .await?;
            }
            nodes[1]
                .net
                .gossip
                .block_store
                .wait_until_persisted(ctx, suffix.last().unwrap().number())
                .await?;

            // Check that the expected block range is actually stored.
            assert_eq!(setup.blocks[8..10], dump(ctx, &stores[1]).await);
        }
        Ok(())
    })
    .await
    .unwrap();
}
