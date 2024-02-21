//! High-level tests for `Executor`.

use super::*;
use tracing::Instrument as _;
use zksync_concurrency::{
    testonly::{abort_on_panic, set_timeout},
    time,
};
use zksync_consensus_bft as bft;
use zksync_consensus_network::testonly::{new_configs, new_fullnode};
use zksync_consensus_roles::validator::{testonly::Setup, BlockNumber};
use zksync_consensus_storage::{
    self as storage,
    testonly::{in_memory, new_store},
    BlockStore, PersistentBlockStore as _,
};

fn make_executor(cfg: &network::Config, block_store: Arc<BlockStore>) -> Executor {
    Executor {
        config: Config {
            server_addr: *cfg.server_addr,
            public_addr: cfg.public_addr,
            max_payload_size: usize::MAX,
            node_key: cfg.gossip.key.clone(),
            gossip_dynamic_inbound_limit: cfg.gossip.dynamic_inbound_limit,
            gossip_static_inbound: cfg.gossip.static_inbound.clone(),
            gossip_static_outbound: cfg.gossip.static_outbound.clone(),
        },
        block_store,
        validator: cfg.validator_key.as_ref().map(|key| Validator {
            key: key.clone(),
            replica_store: Box::new(in_memory::ReplicaStore::default()),
            payload_manager: Box::new(bft::testonly::RandomPayload(1000)),
        }),
    }
}

#[tokio::test]
async fn executing_single_validator() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(make_executor(&cfgs[0], store.clone()).run(ctx));
        store.wait_until_persisted(ctx, BlockNumber(5)).await?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn executing_validator_and_full_node() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        // Spawn validator.
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(make_executor(&cfgs[0], store).run(ctx));

        // Spawn full node.
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(make_executor(&new_fullnode(rng, &cfgs[0]), store.clone()).run(ctx));

        // Wait for blocks in full node store.
        store.wait_until_persisted(ctx, BlockNumber(5)).await?;
        Ok(())
    })
    .await
    .unwrap();
}

/*
/// * finalize some blocks
/// * revert bunch of blocks
/// * restart validators and make sure that new blocks get produced
/// * start additional full node to make sure that it can sync blocks from before the fork
#[tokio::test]
async fn test_block_revert() {
    abort_on_panic();
    let _guard = set_timeout(time::Duration::seconds(10));

    let ctx = &ctx::test_root(&ctx::AffineClock::new(10.));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 2);
    let first = setup.next();
    let cfgs = new_configs(rng, &setup, 1);
    // Persistent stores for the validators.
    let mut persistent_stores: Vec<_> = cfgs
        .iter()
        .map(|_| in_memory::BlockStore::new(setup.genesis.clone()))
        .collect();

    // Make validators produce some blocks.
    scope::run!(ctx, |ctx, s| async {
        let mut stores = vec![];
        for i in 0..cfgs.len() {
            let (store, runner) = BlockStore::new(ctx, Box::new(persistent_stores[i].clone()))
                .await
                .unwrap();
            s.spawn_bg(runner.run(ctx));
            s.spawn_bg(make_executor(&cfgs[i], store.clone()).run(ctx));
            stores.push(store);
        }
        for s in stores {
            s.wait_until_persisted(ctx, BlockNumber(first.0 + 6))
                .await?;
        }
        Ok(())
    })
    .await
    .unwrap();

    tracing::info!("Revert blocks");
    let first = BlockNumber(first.0 + 3);
    let fork = validator::Fork {
        number: setup.genesis.fork.number.next(),
        first_block: first,
        first_parent: persistent_stores[0]
            .block(ctx, first)
            .await
            .unwrap()
            .header()
            .parent,
    };
    let mut genesis = setup.genesis.clone();
    genesis.fork = fork.clone();
    // Update configs and persistent storage.
    for store in &mut persistent_stores {
        *store = store.fork(fork.clone()).unwrap();
    }

    let last_block = BlockNumber(first.0 + 8);
    scope::run!(ctx, |ctx, s| async {
        tracing::info!("Make validators produce blocks on the new fork.");
        let mut stores = vec![];
        for i in 0..cfgs.len() {
            let (store, runner) = BlockStore::new(ctx, Box::new(persistent_stores[i].clone()))
                .await
                .unwrap();
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            s.spawn_bg(
                make_executor(&cfgs[i], store.clone())
                    .run(ctx)
                    .instrument(tracing::info_span!("node", i)),
            );
            stores.push(store);
        }

        tracing::info!("Spawn a new node with should fetch blocks from both new and old fork");
        let (store, runner) = new_store(ctx, &genesis).await;
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("fullnode")));
        s.spawn_bg(
            make_executor(&new_fullnode(rng, &cfgs[0]), store.clone())
                .run(ctx)
                .instrument(tracing::info_span!("fullnode")),
        );
        store.wait_until_persisted(ctx, last_block).await?;
        storage::testonly::verify(ctx, &store)
            .await
            .context("verify(storage)")?;
        Ok(())
    })
    .await
    .unwrap();
}
*/
