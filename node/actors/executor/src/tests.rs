//! High-level tests for `Executor`.
use super::*;
use zksync_concurrency::testonly::abort_on_panic;
use zksync_consensus_bft as bft;
use zksync_consensus_network::testonly::{new_configs, new_fullnode};
use zksync_consensus_roles::validator::{testonly::Setup, BlockNumber};
use zksync_consensus_storage::{
    testonly::{in_memory, new_store, new_store_with_first},
    BlockStore,
};

fn config(cfg: &network::Config) -> Config {
    Config {
        server_addr: *cfg.server_addr,
        public_addr: cfg.public_addr,
        max_payload_size: usize::MAX,
        node_key: cfg.gossip.key.clone(),
        gossip_dynamic_inbound_limit: cfg.gossip.dynamic_inbound_limit,
        gossip_static_inbound: cfg.gossip.static_inbound.clone(),
        gossip_static_outbound: cfg.gossip.static_outbound.clone(),
    }
}

fn validator(
    cfg: &network::Config,
    block_store: Arc<BlockStore>,
    replica_store: impl ReplicaStore,
) -> Executor {
    Executor {
        config: config(cfg),
        block_store,
        validator: Some(Validator {
            key: cfg.validator_key.clone().unwrap(),
            replica_store: Box::new(replica_store),
            payload_manager: Box::new(bft::testonly::RandomPayload(1000)),
        }),
    }
}

fn fullnode(cfg: &network::Config, block_store: Arc<BlockStore>) -> Executor {
    Executor {
        config: config(cfg),
        block_store,
        validator: None,
    }
}

#[tokio::test]
async fn test_single_validator() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        let replica_store = in_memory::ReplicaStore::default();
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(validator(&cfgs[0], store.clone(), replica_store).run(ctx));
        store.wait_until_persisted(ctx, BlockNumber(5)).await?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_fullnode_syncing_from_validator() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        // Spawn validator.
        let replica_store = in_memory::ReplicaStore::default();
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(validator(&cfgs[0], store, replica_store).run(ctx));

        // Spawn full node.
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(fullnode(&new_fullnode(rng, &cfgs[0]), store.clone()).run(ctx));

        // Wait for blocks in full node store.
        store
            .wait_until_persisted(ctx, setup.genesis.fork.first_block + 5)
            .await?;
        Ok(())
    })
    .await
    .unwrap();
}

/// Test in which validator is syncing missing blocks from a full node before producing blocks.
#[tokio::test]
async fn test_validator_syncing_from_fullnode() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        // Spawn full node.
        let (node_store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(fullnode(&new_fullnode(rng, &cfgs[0]), node_store.clone()).run(ctx));

        // Run validator and produce some blocks.
        // Wait for the blocks to be fetched by the full node.
        let replica_store = in_memory::ReplicaStore::default();
        scope::run!(ctx, |ctx, s| async {
            let (store, runner) = new_store(ctx, &setup.genesis).await;
            s.spawn_bg(runner.run(ctx));
            s.spawn_bg(validator(&cfgs[0], store, replica_store.clone()).run(ctx));
            node_store
                .wait_until_persisted(ctx, setup.genesis.fork.first_block + 4)
                .await?;
            Ok(())
        })
        .await
        .unwrap();

        // Restart the validator with empty store (but preserved replica state) and non-trivial
        // `store.state.first`.
        // Validator should fetch the past blocks from the full node before producing next blocks.
        let last_block = node_store
            .subscribe()
            .borrow()
            .last
            .as_ref()
            .unwrap()
            .header()
            .number;
        let (store, runner) =
            new_store_with_first(ctx, &setup.genesis, setup.genesis.fork.first_block + 2).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(validator(&cfgs[0], store, replica_store).run(ctx));
        node_store.wait_until_persisted(ctx, last_block + 3).await?;

        Ok(())
    })
    .await
    .unwrap();
}
