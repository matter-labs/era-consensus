use rand::Rng as _;
use zksync_concurrency::testonly::abort_on_panic;
use zksync_consensus_engine::{testonly::TestEngine, EngineManager};
use zksync_consensus_network::testonly::{new_configs, new_fullnode};
use zksync_consensus_roles::validator::{testonly::Setup, BlockNumber};

use super::*;

fn validator(cfg: &network::Config, engine_manager: Arc<EngineManager>) -> Executor {
    let config = Config {
        build_version: None,
        server_addr: *cfg.server_addr,
        public_addr: cfg.public_addr.clone(),
        max_payload_size: usize::MAX,
        view_timeout: time::Duration::milliseconds(1000),
        gossip_dynamic_inbound_limit: cfg.gossip.dynamic_inbound_limit,
        gossip_static_inbound: cfg.gossip.static_inbound.clone(),
        gossip_static_outbound: cfg.gossip.static_outbound.clone(),
        rpc: cfg.rpc.clone(),
        node_key: cfg.gossip.key.clone(),
        validator_key: cfg.validator_key.clone(),
        debug_page: None,
    };

    Executor {
        config,
        engine_manager,
    }
}

fn full_node(cfg: &network::Config, engine_manager: Arc<EngineManager>) -> Executor {
    let config = Config {
        build_version: None,
        server_addr: *cfg.server_addr,
        public_addr: cfg.public_addr.clone(),
        max_payload_size: usize::MAX,
        view_timeout: time::Duration::milliseconds(1000),
        gossip_dynamic_inbound_limit: cfg.gossip.dynamic_inbound_limit,
        gossip_static_inbound: cfg.gossip.static_inbound.clone(),
        gossip_static_outbound: cfg.gossip.static_outbound.clone(),
        rpc: cfg.rpc.clone(),
        node_key: cfg.gossip.key.clone(),
        validator_key: None,
        debug_page: None,
    };

    Executor {
        config,
        engine_manager,
    }
}

#[tokio::test]
async fn test_single_validator() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);

    let res = scope::run!(ctx, |ctx, s| async {
        // Spawn validator.
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        s.spawn_bg(validator(&cfgs[0], engine.manager.clone()).run(ctx));

        // Wait for blocks to get finalized by this validator.
        engine
            .manager
            .wait_until_persisted(ctx, BlockNumber(5))
            .await?;

        Ok(())
    })
    .await;

    // Just ignore the "canceled" error and treat it as success
    if let Err(e) = &res {
        if e.to_string() == "canceled" {
            return;
        }
        println!("err: {:?}", e);
        assert!(false, "Test failed with error: {:?}", e);
    }
}

#[tokio::test]
async fn test_many_validators() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 3);
    let cfgs = new_configs(rng, &setup, 1);

    let res = scope::run!(ctx, |ctx, s| async {
        for cfg in cfgs {
            // Spawn validator.
            let engine = TestEngine::new(ctx, &setup).await;
            s.spawn_bg(engine.runner.run(ctx));
            s.spawn_bg(validator(&cfg, engine.manager.clone()).run(ctx));

            // Spawn a task waiting for blocks to get finalized and delivered to this validator.
            s.spawn(async {
                let manager = engine.manager;
                manager.wait_until_persisted(ctx, BlockNumber(5)).await?;
                Ok(())
            });
        }

        Ok(())
    })
    .await;

    // Just ignore the "canceled" error and treat it as success
    if let Err(e) = &res {
        if e.to_string() == "canceled" {
            return;
        }
        println!("err: {:?}", e);
        assert!(false, "Test failed with error: {:?}", e);
    }
}

#[tokio::test]
async fn test_inactive_validator() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);

    let res = scope::run!(ctx, |ctx, s| async {
        // Spawn validator.
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        s.spawn_bg(validator(&cfgs[0], engine.manager.clone()).run(ctx));

        // Spawn a validator node, which doesn't belong to the consensus.
        // Therefore it should behave just like a fullnode.
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        let mut cfg = new_fullnode(rng, &cfgs[0]);
        cfg.validator_key = Some(rng.gen());
        s.spawn_bg(validator(&cfg, engine.manager.clone()).run(ctx));

        // Wait for blocks in inactive validator's store.
        engine
            .manager
            .wait_until_persisted(ctx, setup.first_block() + 5)
            .await?;

        Ok(())
    })
    .await;

    // Just ignore the "canceled" error and treat it as success
    if let Err(e) = &res {
        if e.to_string() == "canceled" {
            return;
        }
    }
}

#[tokio::test]
async fn test_fullnode_syncing_from_validator() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);

    let res = scope::run!(ctx, |ctx, s| async {
        // Spawn validator.
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        s.spawn_bg(validator(&cfgs[0], engine.manager.clone()).run(ctx));

        // Spawn full node.
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        s.spawn_bg(full_node(&new_fullnode(rng, &cfgs[0]), engine.manager.clone()).run(ctx));

        // Wait for blocks in full node store.
        engine
            .manager
            .wait_until_persisted(ctx, setup.first_block() + 5)
            .await?;

        Ok(())
    })
    .await;

    // Just ignore the "canceled" error and treat it as success
    if let Err(e) = &res {
        if e.to_string() == "canceled" {
            return;
        }
    }
}

/// Test in which validator is syncing missing blocks from a full node before producing blocks.
#[tokio::test]
async fn test_validator_syncing_from_fullnode() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);

    let res = scope::run!(ctx, |ctx, s| async {
        // Spawn full node.
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        s.spawn_bg(full_node(&new_fullnode(rng, &cfgs[0]), engine.manager.clone()).run(ctx));

        // Spawn first validator.
        let engine_2 = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine_2.runner.run(ctx));

        // Run first validator and produce some blocks.
        let _ = scope::run!(ctx, |ctx, s| async {
            // Run validator here so it stops after we produce the blocks.
            s.spawn_bg(validator(&cfgs[0], engine_2.manager.clone()).run(ctx));

            // Wait for validator to produce some blocks.
            engine_2
                .manager
                .wait_until_persisted(ctx, setup.first_block() + 4)
                .await?;

            // Wait for blocks in full node store.
            engine
                .manager
                .wait_until_persisted(ctx, setup.first_block() + 4)
                .await?;

            Ok(())
        })
        .await;

        // Start a new validator with non-trivial first block.
        // Validator should fetch the past blocks from the full node before producing next blocks.
        let engine_3 = TestEngine::new_with_first_block(ctx, &setup, setup.first_block() + 2).await;
        s.spawn_bg(engine_3.runner.run(ctx));
        s.spawn_bg(validator(&cfgs[0], engine_3.manager.clone()).run(ctx));

        // Wait for blocks in validator store.
        engine_3
            .manager
            .wait_until_persisted(ctx, setup.first_block() + 4)
            .await?;

        Ok(())
    })
    .await;

    // Just ignore the "canceled" error and treat it as success
    if let Err(e) = &res {
        if e.to_string() == "canceled" {
            return;
        }
    }
}
