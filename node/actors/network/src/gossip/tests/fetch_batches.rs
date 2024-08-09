//! Unit tests of `get_batch` RPC.
use crate::{gossip, mux, rpc};
use assert_matches::assert_matches;
use tracing::Instrument as _;
use zksync_concurrency::{ctx, limiter, scope, testonly::abort_on_panic};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::{testonly::TestMemoryStorage, BatchStoreState};

#[tokio::test]
async fn test_simple() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::Setup::new(rng, 1);
    setup.push_batches(rng, 2);
    let mut cfg = crate::testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.rpc.push_batch_store_state_rate = limiter::Rate::INF;
    cfg.rpc.get_batch_rate = limiter::Rate::INF;
    cfg.rpc.get_batch_timeout = None;
    cfg.validator_key = None;

    scope::run!(ctx, |ctx, s| async {
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        let (_node, runner) = crate::testonly::Instance::new(
            cfg.clone(),
            store.blocks.clone(),
            store.batches.clone(),
        );
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));

        let (conn, runner) = gossip::testonly::connect(ctx, &cfg, setup.genesis.hash())
            .await
            .unwrap();
        s.spawn_bg(async {
            assert_matches!(runner.run(ctx).await, Err(mux::RunError::Canceled(_)));
            Ok(())
        });

        tracing::info!("Store is empty so requesting a batch should return an empty response.");
        let mut stream = conn.open_client::<rpc::get_batch::Rpc>(ctx).await.unwrap();
        stream
            .send(ctx, &rpc::get_batch::Req(setup.batches[1].number))
            .await
            .unwrap();
        let resp = stream.recv(ctx).await.unwrap();
        assert_eq!(resp.0, None);

        tracing::info!("Insert a batch.");
        store
            .batches
            .queue_batch(ctx, setup.batches[0].clone(), setup.genesis.clone())
            .await
            .unwrap();
        loop {
            let mut stream = conn
                .open_server::<rpc::push_batch_store_state::Rpc>(ctx)
                .await
                .unwrap();
            let state = stream.recv(ctx).await.unwrap();
            stream.send(ctx, &()).await.unwrap();
            if state.0.contains(setup.batches[0].number) {
                tracing::info!("peer reported to have a batch");
                break;
            }
        }
        tracing::info!("fetch that batch.");

        let mut stream = conn.open_client::<rpc::get_batch::Rpc>(ctx).await.unwrap();
        stream
            .send(ctx, &rpc::get_batch::Req(setup.batches[0].number))
            .await
            .unwrap();
        let resp = stream.recv(ctx).await.unwrap();
        assert_eq!(resp.0, Some(setup.batches[0].clone()));

        tracing::info!("Inform the peer that we have {}", setup.batches[1].number);
        let mut stream = conn
            .open_client::<rpc::push_batch_store_state::Rpc>(ctx)
            .await
            .unwrap();
        stream
            .send(
                ctx,
                &rpc::push_batch_store_state::Req(BatchStoreState {
                    first: setup.batches[1].number,
                    last: Some(setup.batches[1].number),
                }),
            )
            .await
            .unwrap();
        stream.recv(ctx).await.unwrap();

        tracing::info!("Wait for the client to request that batch");
        let mut stream = conn.open_server::<rpc::get_batch::Rpc>(ctx).await.unwrap();
        let req = stream.recv(ctx).await.unwrap();
        assert_eq!(req.0, setup.batches[1].number);

        tracing::info!("Return the requested batch");
        stream
            .send(ctx, &rpc::get_batch::Resp(Some(setup.batches[1].clone())))
            .await
            .unwrap();

        tracing::info!("Wait for the client to store that batch");
        store
            .batches
            .wait_until_persisted(ctx, setup.batches[1].number)
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
    let mut setup = validator::testonly::Setup::new(rng, 1);
    setup.push_batches(rng, 10);
    let mut cfg = crate::testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.rpc.push_batch_store_state_rate = limiter::Rate::INF;
    cfg.rpc.get_batch_rate = limiter::Rate::INF;
    cfg.rpc.get_batch_timeout = None;
    cfg.validator_key = None;

    scope::run!(ctx, |ctx, s| async {
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        let (_node, runner) = crate::testonly::Instance::new(
            cfg.clone(),
            store.blocks.clone(),
            store.batches.clone(),
        );
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
                .open_client::<rpc::push_batch_store_state::Rpc>(ctx)
                .await
                .unwrap();
            stream
                .send(
                    ctx,
                    &rpc::push_batch_store_state::Req(BatchStoreState {
                        first: setup.batches[0].number,
                        last: Some(setup.batches.last().unwrap().number),
                    }),
                )
                .await
                .unwrap();
            stream.recv(ctx).await.unwrap();
            conns.push(conn);
        }

        // Receive a bunch of concurrent requests on various connections.
        let mut streams = vec![];
        for (i, batch) in setup.batches.iter().enumerate() {
            let mut stream = conns[i % conns.len()]
                .open_server::<rpc::get_batch::Rpc>(ctx)
                .await
                .unwrap();
            let req = stream.recv(ctx).await.unwrap();
            assert_eq!(req.0, batch.number);
            streams.push(stream);
        }

        // Respond to the requests.
        for (i, stream) in streams.into_iter().enumerate() {
            stream
                .send(ctx, &rpc::get_batch::Resp(Some(setup.batches[i].clone())))
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
    let mut setup = validator::testonly::Setup::new(rng, 1);
    setup.push_batches(rng, 2);
    let mut cfg = crate::testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.rpc.push_batch_store_state_rate = limiter::Rate::INF;
    cfg.rpc.get_batch_rate = limiter::Rate::INF;
    cfg.rpc.get_batch_timeout = None;
    cfg.validator_key = None;

    scope::run!(ctx, |ctx, s| async {
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        let (_node, runner) = crate::testonly::Instance::new(
            cfg.clone(),
            store.blocks.clone(),
            store.batches.clone(),
        );
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));

        let state = rpc::push_batch_store_state::Req(BatchStoreState {
            first: setup.batches[0].number,
            last: Some(setup.batches[0].number),
        });

        for resp in [
            // Empty response even though we declared to have the batch.
            None,
            // Wrong batch.
            Some(setup.batches[1].clone()),
            // // Malformed batch.
            // {
            //     let mut b = setup.batches[0].clone();
            //     b.justification = rng.gen();
            //     Some(b)
            // },
        ] {
            tracing::info!("bad response = {resp:?}");

            tracing::info!("Connect to peer");
            let (conn, runner) = gossip::testonly::connect(ctx, &cfg, setup.genesis.hash())
                .await
                .unwrap();
            let conn_task = s.spawn_bg(async { Ok(runner.run(ctx).await) });

            tracing::info!("Inform the peer about the batch that we possess");
            let mut stream = conn
                .open_client::<rpc::push_batch_store_state::Rpc>(ctx)
                .await
                .unwrap();
            stream.send(ctx, &state).await.unwrap();
            stream.recv(ctx).await.unwrap();

            tracing::info!("Wait for the client to request that batch");
            let mut stream = conn.open_server::<rpc::get_batch::Rpc>(ctx).await.unwrap();
            let req = stream.recv(ctx).await.unwrap();
            assert_eq!(req.0, setup.batches[0].number);

            tracing::info!("Return a bad response");
            stream.send(ctx, &rpc::get_batch::Resp(resp)).await.unwrap();

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
    let mut setup = validator::testonly::Setup::new(rng, 1);
    setup.push_batches(rng, 1);
    let mut cfg = crate::testonly::new_configs(rng, &setup, 0)[0].clone();
    cfg.rpc.push_batch_store_state_rate = limiter::Rate::INF;
    cfg.rpc.get_batch_rate = limiter::Rate::INF;
    cfg.rpc.get_batch_timeout = None;
    cfg.validator_key = None;

    scope::run!(ctx, |ctx, s| async {
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        let (_node, runner) = crate::testonly::Instance::new(
            cfg.clone(),
            store.blocks.clone(),
            store.batches.clone(),
        );
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));

        let state = rpc::push_batch_store_state::Req(BatchStoreState {
            first: setup.batches[0].number,
            last: Some(setup.batches[0].number),
        });

        tracing::info!("establish a bunch of connections");
        let mut conns = vec![];
        for _ in 0..4 {
            let (conn, runner) = gossip::testonly::connect(ctx, &cfg, setup.genesis.hash())
                .await
                .unwrap();
            let task = s.spawn_bg(async { Ok(runner.run(ctx).await) });
            let mut stream = conn
                .open_client::<rpc::push_batch_store_state::Rpc>(ctx)
                .await
                .unwrap();
            stream.send(ctx, &state).await.unwrap();
            stream.recv(ctx).await.unwrap();
            conns.push((conn, task));
        }

        for (conn, task) in conns {
            tracing::info!("Wait for the client to request a batch");
            let mut stream = conn.open_server::<rpc::get_batch::Rpc>(ctx).await.unwrap();
            let req = stream.recv(ctx).await.unwrap();
            assert_eq!(req.0, setup.batches[0].number);

            tracing::info!("Return a bad response");
            stream.send(ctx, &rpc::get_batch::Resp(None)).await.unwrap();

            tracing::info!("Wait for the peer to drop the connection");
            assert_matches!(task.join(ctx).await.unwrap(), Err(mux::RunError::Closed));
        }

        Ok(())
    })
    .await
    .unwrap();
}
