//! Unit tests of `get_block` RPC.
use assert_matches::assert_matches;
use rand::Rng as _;
use tracing::Instrument as _;
use zksync_concurrency::{ctx, limiter, scope, testonly::abort_on_panic};
use zksync_consensus_engine::{
    testonly::{in_memory, TestEngine},
    BlockStoreState, EngineInterface as _, EngineManager,
};
use zksync_consensus_roles::validator;

use crate::{gossip, mux, rpc};

#[tokio::test]
async fn test_simple() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new_without_pregenesis(rng, 1);
    setup.push_blocks_v1(rng, 2);
    let mut cfg = crate::testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
    cfg.rpc.get_block_rate = limiter::Rate::INF;
    cfg.rpc.get_block_timeout = None;
    cfg.validator_key = None;

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));

        let (_node, runner) = crate::testonly::Instance::new(cfg.clone(), engine.manager.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));

        let (conn, runner) = gossip::testonly::connect(ctx, &cfg, setup.genesis.hash())
            .await
            .unwrap();
        s.spawn_bg(async {
            assert_matches!(runner.run(ctx).await, Err(mux::RunError::Canceled(_)));
            Ok(())
        });

        tracing::info!("Store is empty so requesting a block should return an empty response.");
        let mut stream = conn.open_client::<rpc::get_block::Rpc>(ctx).await.unwrap();
        stream
            .send(ctx, &rpc::get_block::Req(setup.blocks[0].number()))
            .await
            .unwrap();
        let resp = stream.recv(ctx).await.unwrap();
        assert_eq!(resp.0, None);

        tracing::info!("Insert a block.");
        engine
            .manager
            .queue_block(
                ctx,
                setup.blocks[0].clone(),
                setup.genesis.validators_schedule.as_ref().unwrap(),
            )
            .await
            .unwrap();
        loop {
            let mut stream = conn
                .open_server::<rpc::push_block_store_state::Rpc>(ctx)
                .await
                .unwrap();
            let resp = stream.recv(ctx).await.unwrap();
            stream.send(ctx, &()).await.unwrap();
            if resp.state.contains(setup.blocks[0].number()) {
                tracing::info!("peer reported to have a block");
                break;
            }
        }
        tracing::info!("fetch that block.");
        let mut stream = conn.open_client::<rpc::get_block::Rpc>(ctx).await.unwrap();
        stream
            .send(ctx, &rpc::get_block::Req(setup.blocks[0].number()))
            .await
            .unwrap();
        let resp = stream.recv(ctx).await.unwrap();
        assert_eq!(resp.0, Some(setup.blocks[0].clone()));

        tracing::info!("Inform the peer that we have {}", setup.blocks[1].number());
        let mut stream = conn
            .open_client::<rpc::push_block_store_state::Rpc>(ctx)
            .await
            .unwrap();
        stream
            .send(
                ctx,
                &rpc::push_block_store_state::Req {
                    state: BlockStoreState {
                        first: setup.blocks[1].number(),
                        last: Some((&setup.blocks[1]).into()),
                    },
                },
            )
            .await
            .unwrap();
        stream.recv(ctx).await.unwrap();

        tracing::info!("Wait for the client to request that block");
        let mut stream = conn.open_server::<rpc::get_block::Rpc>(ctx).await.unwrap();
        let req = stream.recv(ctx).await.unwrap();
        assert_eq!(req.0, setup.blocks[1].number());

        tracing::info!("Return the requested block");
        stream
            .send(ctx, &rpc::get_block::Resp(Some(setup.blocks[1].clone())))
            .await
            .unwrap();

        tracing::info!("Wait for the client to store that block");
        engine
            .manager
            .wait_until_persisted(ctx, setup.blocks[1].number())
            .await
            .unwrap();

        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_concurrent_requests() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new_without_pregenesis(rng, 1);
    setup.push_blocks_v1(rng, 10);
    let mut cfg = crate::testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
    cfg.rpc.get_block_rate = limiter::Rate::INF;
    cfg.rpc.get_block_timeout = None;
    cfg.validator_key = None;
    cfg.max_block_queue_size = setup.blocks.len();

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        let (_node, runner) = crate::testonly::Instance::new(cfg.clone(), engine.manager.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));

        let mut conns = vec![];
        for _ in 0..4 {
            let (conn, runner) = gossip::testonly::connect(ctx, &cfg, setup.genesis.hash())
                .await
                .unwrap();
            s.spawn_bg(async {
                assert_matches!(runner.run(ctx).await, Err(mux::RunError::Canceled(_)));
                Ok(())
            });
            let mut stream = conn
                .open_client::<rpc::push_block_store_state::Rpc>(ctx)
                .await
                .unwrap();
            stream
                .send(
                    ctx,
                    &rpc::push_block_store_state::Req {
                        state: BlockStoreState {
                            first: setup.blocks[0].number(),
                            last: Some(setup.blocks.last().unwrap().into()),
                        },
                    },
                )
                .await
                .unwrap();
            stream.recv(ctx).await.unwrap();
            conns.push(conn);
        }

        // Receive a bunch of concurrent requests on various connections.
        let mut streams = vec![];
        for (i, block) in setup.blocks.iter().enumerate() {
            let mut stream = conns[i % conns.len()]
                .open_server::<rpc::get_block::Rpc>(ctx)
                .await
                .unwrap();
            let req = stream.recv(ctx).await.unwrap();
            assert_eq!(req.0, block.number());
            streams.push(stream);
        }

        // Respond to the requests.
        for (i, stream) in streams.into_iter().enumerate() {
            stream
                .send(ctx, &rpc::get_block::Resp(Some(setup.blocks[i].clone())))
                .await
                .unwrap();
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_bad_responses() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new_without_pregenesis(rng, 1);
    setup.push_blocks_v1(rng, 2);
    let mut cfg = crate::testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
    cfg.rpc.get_block_rate = limiter::Rate::INF;
    cfg.rpc.get_block_timeout = None;
    cfg.validator_key = None;

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        let (_node, runner) = crate::testonly::Instance::new(cfg.clone(), engine.manager.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));

        let state = rpc::push_block_store_state::Req {
            state: BlockStoreState {
                first: setup.blocks[0].number(),
                last: Some((&setup.blocks[0]).into()),
            },
        };

        for resp in [
            // Empty response even though we declared to have the block.
            None,
            // Wrong block.
            Some(setup.blocks[1].clone()),
            // Malformed block.
            {
                let validator::Block::FinalV1(mut b) = setup.blocks[0].clone() else {
                    panic!();
                };
                b.justification = rng.gen();
                Some(b.into())
            },
        ] {
            tracing::info!("bad response = {resp:?}");

            tracing::info!("Connect to peer");
            let (conn, runner) = gossip::testonly::connect(ctx, &cfg, setup.genesis.hash())
                .await
                .unwrap();
            let conn_task = s.spawn_bg(async { Ok(runner.run(ctx).await) });

            tracing::info!("Inform the peer about the block that we possess");
            let mut stream = conn
                .open_client::<rpc::push_block_store_state::Rpc>(ctx)
                .await
                .unwrap();
            stream.send(ctx, &state).await.unwrap();
            stream.recv(ctx).await.unwrap();

            tracing::info!("Wait for the client to request that block");
            let mut stream = conn.open_server::<rpc::get_block::Rpc>(ctx).await.unwrap();
            let req = stream.recv(ctx).await.unwrap();
            assert_eq!(req.0, setup.blocks[0].number());

            tracing::info!("Return a bad response");
            stream.send(ctx, &rpc::get_block::Resp(resp)).await.unwrap();

            tracing::info!("Wait for the peer to drop the connection");
            assert_matches!(
                conn_task.join(ctx).await.unwrap(),
                Err(mux::RunError::Closed)
            );
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_retry() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new_without_pregenesis(rng, 1);
    setup.push_blocks_v1(rng, 1);
    let mut cfg = crate::testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
    cfg.rpc.get_block_rate = limiter::Rate::INF;
    cfg.rpc.get_block_timeout = None;
    cfg.validator_key = None;

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        let (_node, runner) = crate::testonly::Instance::new(cfg.clone(), engine.manager.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));

        let state = rpc::push_block_store_state::Req {
            state: BlockStoreState {
                first: setup.blocks[0].number(),
                last: Some((&setup.blocks[0]).into()),
            },
        };

        tracing::info!("establish a bunch of connections");
        let mut conns = vec![];
        for _ in 0..4 {
            let (conn, runner) = gossip::testonly::connect(ctx, &cfg, setup.genesis.hash())
                .await
                .unwrap();
            let task = s.spawn_bg(async { Ok(runner.run(ctx).await) });
            let mut stream = conn
                .open_client::<rpc::push_block_store_state::Rpc>(ctx)
                .await
                .unwrap();
            stream.send(ctx, &state).await.unwrap();
            stream.recv(ctx).await.unwrap();
            conns.push((conn, task));
        }

        for (conn, task) in conns {
            tracing::info!("Wait for the client to request a block");
            let mut stream = conn.open_server::<rpc::get_block::Rpc>(ctx).await.unwrap();
            let req = stream.recv(ctx).await.unwrap();
            assert_eq!(req.0, setup.blocks[0].number());

            tracing::info!("Return a bad response");
            stream.send(ctx, &rpc::get_block::Resp(None)).await.unwrap();

            tracing::info!("Wait for the peer to drop the connection");
            assert_matches!(task.join(ctx).await.unwrap(), Err(mux::RunError::Closed));
        }

        Ok(())
    })
    .await
    .unwrap();
}

/// Test checking that if storage is truncated,
/// then the node announces that to peers.
#[tokio::test]
async fn test_announce_truncated_block_range() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new_without_pregenesis(rng, 1);
    setup.push_blocks_v1(rng, 10);
    let mut cfg = crate::testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.rpc.push_block_store_state_rate = limiter::Rate::INF;
    cfg.rpc.get_block_rate = limiter::Rate::INF;
    cfg.rpc.get_block_timeout = None;
    cfg.validator_key = None;

    scope::run!(ctx, |ctx, s| async {
        // Build a custom persistent store, so that we can tweak it later.
        let mut engine = in_memory::Engine::new_random(&setup, setup.genesis.first_block);
        let (manager, runner) = EngineManager::new(ctx, Box::new(engine.clone())).await?;
        s.spawn_bg(runner.run(ctx));
        let (_node, runner) = crate::testonly::Instance::new(cfg.clone(), manager);
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));
        // Fill in all the blocks.
        for b in &setup.blocks {
            engine.queue_next_block(ctx, b.clone()).await?;
        }

        // Connect to the node.
        let (conn, runner) = gossip::testonly::connect(ctx, &cfg, setup.genesis.hash())
            .await
            .unwrap();
        s.spawn_bg(async {
            assert_matches!(runner.run(ctx).await, Err(mux::RunError::Canceled(_)));
            Ok(())
        });

        let mut first = setup.genesis.first_block;
        loop {
            tracing::info!("Truncate up to {first}");
            engine.truncate(first);
            first = first + 3;

            // Listen to `PublicBlockStoreState` messages.
            // Until it is consistent with storage.
            loop {
                let mut stream = conn
                    .open_server::<rpc::push_block_store_state::Rpc>(ctx)
                    .await?;
                let resp = stream.recv(ctx).await.unwrap();
                stream.send(ctx, &()).await.unwrap();
                if resp.state == *engine.persisted().borrow() {
                    break;
                }
            }

            // If there are no blocks left, we are done.
            let left = engine.persisted().borrow().clone();
            if left.next() <= left.first {
                break;
            }
        }
        Ok(())
    })
    .await
    .unwrap();
}
