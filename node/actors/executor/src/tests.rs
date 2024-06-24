//! High-level tests for `Executor`.
use super::*;
use rand::Rng as _;
use tracing::Instrument as _;
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
        public_addr: cfg.public_addr.clone(),
        max_payload_size: usize::MAX,
        node_key: cfg.gossip.key.clone(),
        gossip_dynamic_inbound_limit: cfg.gossip.dynamic_inbound_limit,
        gossip_static_inbound: cfg.gossip.static_inbound.clone(),
        gossip_static_outbound: cfg.gossip.static_outbound.clone(),
        rpc: cfg.rpc.clone(),
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
async fn test_many_validators() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 3);
    let cfgs = new_configs(rng, &setup, 1);
    scope::run!(ctx, |ctx, s| async {
        for cfg in cfgs {
            let replica_store = in_memory::ReplicaStore::default();
            let (store, runner) = new_store(ctx, &setup.genesis).await;
            s.spawn_bg(runner.run(ctx));
            s.spawn_bg(validator(&cfg, store.clone(), replica_store).run(ctx));

            // Spawn a task waiting for blocks to get finalized and delivered to this validator.
            s.spawn(async {
                let store = store;
                store.wait_until_persisted(ctx, BlockNumber(5)).await?;
                Ok(())
            });
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_inactive_validator() {
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

        // Spawn a validator node, which doesn't belong to the consensus.
        // Therefore it should behave just like a fullnode.
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        let mut cfg = new_fullnode(rng, &cfgs[0]);
        cfg.validator_key = Some(rng.gen());
        let replica_store = in_memory::ReplicaStore::default();
        s.spawn_bg(validator(&cfg, store.clone(), replica_store).run(ctx));

        // Wait for blocks in inactive validator's store.
        store
            .wait_until_persisted(ctx, setup.genesis.first_block + 5)
            .await?;
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
            .wait_until_persisted(ctx, setup.genesis.first_block + 5)
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
        // Run validator and produce some blocks.
        let replica_store = in_memory::ReplicaStore::default();
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(
                validator(&cfgs[0], store.clone(), replica_store.clone())
                    .run(ctx)
                    .instrument(tracing::info_span!("validator")),
            );
            store
                .wait_until_persisted(ctx, setup.genesis.first_block + 4)
                .await?;
            Ok(())
        })
        .await
        .unwrap();

        // Spawn full node with storage used previously by validator -this ensures that
        // all finalized blocks are available: if we ran a fullnode in parallel to the
        // validator, there would be a race condition between fullnode syncing and validator
        // terminating.
        s.spawn_bg(
            fullnode(&new_fullnode(rng, &cfgs[0]), store.clone())
                .run(ctx)
                .instrument(tracing::info_span!("fullnode")),
        );

        // Restart the validator with empty store (but preserved replica state) and non-trivial
        // `store.state.first`.
        // Validator should fetch the past blocks from the full node before producing next blocks.
        let last_block = store.queued().last.as_ref().unwrap().header().number;
        let (store2, runner) =
            new_store_with_first(ctx, &setup.genesis, setup.genesis.first_block + 2).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(
            validator(&cfgs[0], store2, replica_store)
                .run(ctx)
                .instrument(tracing::info_span!("validator")),
        );

        // Wait for the fullnode to fetch the new blocks.
        store.wait_until_persisted(ctx, last_block + 3).await?;
        Ok(())
    })
    .await
    .unwrap();
}
