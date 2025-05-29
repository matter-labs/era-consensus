use rand::Rng as _;
use zksync_concurrency::testonly::abort_on_panic;
use zksync_consensus_engine::{testonly::TestEngine, EngineManager};
use zksync_consensus_network::testonly::{new_configs, new_fullnode};
use zksync_consensus_roles::validator::{testonly::Setup, BlockNumber, EpochNumber};

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
        panic!("Test failed with error: {:?}", e);
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
        panic!("Test failed with error: {:?}", e);
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
        panic!("Test failed with error: {:?}", e);
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
        panic!("Test failed with error: {:?}", e);
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
        panic!("Test failed with error: {:?}", e);
    }
}

/// Test that the validator schedule rotation feature works as expected.
/// The test can take several seconds up to a minute to run.
#[tokio::test]
async fn test_validator_rotation() {
    abort_on_panic();

    // We want the epoch length to be long enough to make sure that the engine manager has time to
    // fetch each new validator schedule before the end of the previous epoch.
    // We also want the test to end at the middle of an epoch to check that the validator schedule
    // didn't change before the end of the epoch.
    const EPOCH_LENGTH: u64 = 30;
    const TEST_DURATION: u64 = 2 * EPOCH_LENGTH + 5;

    // We need to speed up the clock so that the engine manager can fetch the new validator schedule
    // in time. If the test is flaky, you can try to change the clock speed and/or the epoch length.
    let ctx = &ctx::test_root(&ctx::AffineClock::new(5.));
    let rng = &mut ctx.rng();

    let setup = Setup::new_without_validators_schedule(rng, 8);
    let cfgs = new_configs(rng, &setup, 4);
    let first_block = setup.first_block();

    let res = scope::run!(ctx, |ctx, s| async {
        for cfg in cfgs {
            // Spawn validator.
            let engine = TestEngine::new_with_dynamic_schedule(ctx, &setup, EPOCH_LENGTH).await;
            s.spawn_bg(engine.runner.run(ctx));
            s.spawn_bg(validator(&cfg, engine.manager.clone()).run(ctx));

            // Spawn a task waiting for blocks to get finalized and delivered to this validator.
            s.spawn(async {
                let manager = engine.manager;

                // Wait for the start of each new epoch.
                let mut cur_schedule = None;
                for i in 0..=TEST_DURATION / EPOCH_LENGTH {
                    // Block at the start of the epoch.
                    let block_number = first_block + i * EPOCH_LENGTH;

                    // Wait for the block to be persisted.
                    manager.wait_until_persisted(ctx, block_number).await?;

                    // Get the block and the validator schedule for the epoch.
                    let block = manager.get_block(ctx, block_number).await?.unwrap();
                    let schedule_opt = manager
                        .validator_schedule(EpochNumber(i))
                        .map(|v| v.schedule);

                    // Check that the validator schedule changes and that the epoch number is correct.
                    assert_eq!(block.epoch(), Some(EpochNumber(i)));
                    assert_ne!(cur_schedule, schedule_opt);
                    cur_schedule = schedule_opt;
                }

                // Wait for the end of the test. Note that we are still in the same epoch.
                let epoch_number = TEST_DURATION / EPOCH_LENGTH;
                let block_number = first_block + TEST_DURATION;
                manager.wait_until_persisted(ctx, block_number).await?;

                // Get the block and the validator schedule at the end of the test.
                let block = manager.get_block(ctx, block_number).await?.unwrap();
                let final_schedule = manager
                    .validator_schedule(EpochNumber(epoch_number))
                    .map(|v| v.schedule);

                // Check that the validator schedule didn't change and that the epoch number is correct.
                assert_eq!(block.epoch(), Some(EpochNumber(epoch_number)));
                assert_eq!(cur_schedule, final_schedule);

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
        panic!("Test failed with error: {:?}", e);
    }
}
