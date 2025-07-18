use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::{ctx, error::Wrap as _, net, scope, testonly::abort_on_panic};
use zksync_consensus_engine::testonly::TestEngine;
use zksync_consensus_roles::validator;

use super::*;
use crate::{io, metrics, preface, rpc, testonly};

#[tokio::test]
async fn test_msg_pool_recv() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let mut msgs: Vec<io::ConsensusInputMessage> = (0..20).map(|_| rng.gen()).collect();
    msgs.sort_by_key(|m| m.message.msg.view_number());

    let pool = MsgPool::new();
    let mut recv = pool.subscribe();
    for m in msgs {
        let m = Arc::new(m);
        pool.send(m.clone());
        assert_eq!(m, recv.recv(ctx).await.unwrap());
    }
}

#[tokio::test]
async fn test_one_connection_per_validator() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = validator::testonly::Setup::new(rng, 3);
    let nodes = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        let nodes: Vec<_> = nodes
            .into_iter()
            .enumerate()
            .map(|(i, node)| {
                let (node, runner) = testonly::Instance::new(node, engine.manager.clone());
                s.spawn_bg(runner.run(ctx).instrument(tracing::trace_span!("node", i)));
                node
            })
            .collect();

        tracing::trace!("waiting for all gossip to be established");
        for node in &nodes {
            node.wait_for_gossip_connections().await;
        }

        tracing::trace!("waiting for all connections to be established");
        for node in &nodes {
            node.wait_for_consensus_connections().await;
        }

        tracing::trace!(
            "Impersonate node 1, and try to establish additional connection to node 0. It should \
             close automatically after the handshake."
        );
        let mut stream = preface::connect(
            ctx,
            *nodes[0].cfg().server_addr,
            preface::Endpoint::ConsensusNet,
        )
        .await?;

        handshake::outbound(
            ctx,
            &nodes[1].cfg().validator_key.clone().unwrap(),
            setup.genesis_hash(),
            &mut stream,
            &nodes[0].cfg().validator_key.as_ref().unwrap().public(),
        )
        .await?;
        // The connection is expected to be closed automatically by node 0.
        // The multiplexer runner should exit gracefully.
        let _ = rpc::Service::new().run(ctx, stream).await;
        tracing::trace!(
            "Exiting the main task. Context will get canceled, all the nodes are expected to \
             terminate gracefully"
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_genesis_mismatch() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = validator::testonly::Setup::new(rng, 2);
    let cfgs = testonly::new_configs(rng, &setup, /*gossip_peers=*/ 0);

    scope::run!(ctx, |ctx, s| async {
        let mut listener = cfgs[1]
            .server_addr
            .bind(false)
            .context("server_addr.bind()")?;

        tracing::trace!("Start one node, we will simulate the other one.");
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        let (node, runner) = testonly::Instance::new(cfgs[0].clone(), engine.manager.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::trace_span!("node")));

        tracing::trace!("Populate the validator_addrs of the running node.");
        node.net
            .gossip
            .validator_addrs
            .update(
                setup.validators_schedule(),
                &[Arc::new(setup.validator_keys[1].sign_msg(
                    validator::NetAddress {
                        addr: *cfgs[1].server_addr,
                        version: 0,
                        timestamp: ctx.now_utc(),
                    },
                ))],
            )
            .await
            .unwrap();

        tracing::trace!("Accept a connection with mismatching genesis.");
        let stream = metrics::MeteredStream::accept(ctx, &mut listener)
            .await
            .wrap("accept()")?;
        let (mut stream, endpoint) = preface::accept(ctx, stream)
            .await
            .wrap("preface::accept()")?;
        assert_eq!(endpoint, preface::Endpoint::ConsensusNet);
        tracing::trace!("Expect the handshake to fail");
        let res = handshake::inbound(ctx, &setup.validator_keys[1], rng.gen(), &mut stream).await;
        assert_matches!(res, Err(handshake::Error::GenesisMismatch));

        tracing::trace!("Try to connect to a node with a mismatching genesis.");
        let mut stream =
            preface::connect(ctx, *cfgs[0].server_addr, preface::Endpoint::ConsensusNet)
                .await
                .context("preface::connect")?;
        let res = handshake::outbound(
            ctx,
            &setup.validator_keys[1],
            rng.gen(),
            &mut stream,
            &setup.validator_keys[0].public(),
        )
        .await;
        tracing::trace!(
            "Expect the peer to verify the mismatching Genesis and close the connection."
        );
        assert_matches!(res, Err(handshake::Error::Stream(_)));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_address_change() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.));
    let rng = &mut ctx.rng();

    let setup = validator::testonly::Setup::new(rng, 5);
    let mut cfgs = testonly::new_configs(rng, &setup, 1);
    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        let mut nodes: Vec<_> = cfgs
            .iter()
            .enumerate()
            .map(|(i, cfg)| {
                let (node, runner) = testonly::Instance::new(cfg.clone(), engine.manager.clone());
                s.spawn_bg(runner.run(ctx).instrument(tracing::trace_span!("node", i)));
                node
            })
            .collect();
        for n in &nodes {
            n.wait_for_consensus_connections().await;
        }
        nodes[0].terminate(ctx).await?;

        // All nodes should lose connection to node[0].
        let key0 = nodes[0].cfg().validator_key.as_ref().unwrap().public();
        for node in &nodes {
            node.wait_for_consensus_disconnect(ctx, &key0).await?;
        }

        // Change the address of node[0].
        // Gossip network peers of node[0] won't be able to connect to it.
        // node[0] is expected to connect to its gossip peers.
        // Then it should broadcast its new address and the consensus network
        // should get reconstructed.
        cfgs[0].server_addr = net::tcp::testonly::reserve_listener();
        cfgs[0].public_addr = (*cfgs[0].server_addr).into();

        let (node0, runner) = testonly::Instance::new(cfgs[0].clone(), engine.manager.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::trace_span!("node0")));

        nodes[0] = node0;
        for n in &nodes {
            n.wait_for_consensus_connections().await;
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Test that messages are correctly transmitted over
/// encrypted authenticated multiplexed stream.
#[tokio::test]
async fn test_transmission() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let setup = validator::testonly::Setup::new(rng, 2);
    let cfgs = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        let mut nodes: Vec<_> = cfgs
            .iter()
            .enumerate()
            .map(|(i, cfg)| {
                let (node, runner) = testonly::Instance::new(cfg.clone(), engine.manager.clone());
                let i = ctx::NoCopy(i);
                s.spawn_bg(async {
                    let i = i;
                    runner
                        .run(ctx)
                        .instrument(tracing::trace_span!("node", i = *i))
                        .await
                        .context(*i)
                });
                node
            })
            .collect();

        tracing::trace!("waiting for all connections to be established");
        for n in &mut nodes {
            n.wait_for_consensus_connections().await;
        }
        for i in 0..10 {
            tracing::trace!("message {i}");
            // Construct a message and ensure that view is increasing
            // (otherwise the message could get filtered out).
            let mut want: validator::Signed<validator::v2::ReplicaCommit> = rng.gen();
            want.msg.view.number = validator::ViewNumber(i);
            let want: validator::Signed<validator::ConsensusMsg> = want.cast().unwrap();
            let in_message = io::ConsensusInputMessage {
                message: want.clone(),
            };
            nodes[0].consensus_sender.send(in_message);

            let message = nodes[1].consensus_receiver.recv(ctx).await.unwrap();

            assert_eq!(want, message.msg);
            tracing::trace!("OK");
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Test that messages are retransmitted when a node gets reconnected.
#[tokio::test]
async fn test_retransmission() {
    abort_on_panic();
    // Speed up is needed to make node0 reconnect to node1 fast.
    let ctx = &ctx::test_root(&ctx::AffineClock::new(40.));
    let rng = &mut ctx.rng();

    let setup = validator::testonly::Setup::new(rng, 2);
    let cfgs = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));

        // Spawn the first node.
        let (node0, runner) = testonly::Instance::new(cfgs[0].clone(), engine.manager.clone());
        s.spawn_bg(runner.run(ctx));

        // Make first node broadcast a message.
        let want: validator::Signed<validator::ConsensusMsg> = rng.gen();
        node0.consensus_sender.send(io::ConsensusInputMessage {
            message: want.clone(),
        });

        // Spawn the second node multiple times.
        // Each time the node should reconnect and re-receive the broadcasted consensus message.
        for i in 0..2 {
            tracing::trace!("iteration {i}");
            scope::run!(ctx, |ctx, s| async {
                let (mut node1, runner) =
                    testonly::Instance::new(cfgs[1].clone(), engine.manager.clone());
                s.spawn_bg(runner.run(ctx));

                let message = node1.consensus_receiver.recv(ctx).await.unwrap();

                assert_eq!(want, message.msg);
                tracing::trace!("OK");

                Ok(())
            })
            .await
            .unwrap();
        }
        Ok(())
    })
    .await
    .unwrap();
}
