use super::*;
use crate::{io, metrics, preface, rpc, testonly};
use assert_matches::assert_matches;
use rand::Rng;
use tracing::Instrument as _;
use zksync_concurrency::{ctx, net, scope, testonly::abort_on_panic};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::testonly::new_store;

#[tokio::test]
async fn test_one_connection_per_validator() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = validator::testonly::Setup::new(rng, 3);
    let nodes = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx,s| async {
        let (store,runner) = new_store(ctx,&setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        let nodes : Vec<_> = nodes.into_iter().enumerate().map(|(i,node)| {
            let (node,runner) = testonly::Instance::new(node, store.clone());
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            node
        }).collect();

        tracing::info!("waiting for all gossip to be established");
        for node in &nodes {
            node.wait_for_gossip_connections().await;
        }

        tracing::info!("waiting for all connections to be established");
        for node in &nodes {
            node.wait_for_consensus_connections().await;
        }

        tracing::info!("Impersonate node 1, and try to establish additional connection to node 0. It should close automatically after the handshake.");
        let mut stream = preface::connect(
            ctx,
            *nodes[0].cfg().server_addr,
            preface::Endpoint::ConsensusNet,
        )
        .await?;

        handshake::outbound(
            ctx,
            &nodes[1].cfg().validator_key.clone().unwrap(),
            setup.genesis.hash(),
            &mut stream,
            &nodes[0].cfg().validator_key.as_ref().unwrap().public(),
        )
        .await?;
        // The connection is expected to be closed automatically by node 0.
        // The multiplexer runner should exit gracefully.
        let _ = rpc::Service::new().run(ctx, stream).await;
        tracing::info!(
            "Exiting the main task. Context will get canceled, all the nodes are expected \
             to terminate gracefully"
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
        let mut listener = cfgs[1].server_addr.bind().context("server_addr.bind()")?;

        tracing::info!("Start one node, we will simulate the other one.");
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        let (node, runner) = testonly::Instance::new(cfgs[0].clone(), store.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));

        tracing::info!("Populate the validator_addrs of the running node.");
        node.net
            .gossip
            .validator_addrs
            .update(
                &setup.genesis.validators,
                &[Arc::new(setup.keys[1].sign_msg(validator::NetAddress {
                    addr: *cfgs[1].server_addr,
                    version: 0,
                    timestamp: ctx.now_utc(),
                }))],
            )
            .await
            .unwrap();

        tracing::info!("Accept a connection with mismatching genesis.");
        let stream = metrics::MeteredStream::accept(ctx, &mut listener)
            .await?
            .context("accept()")?;
        let (mut stream, endpoint) = preface::accept(ctx, stream)
            .await
            .context("preface::accept()")?;
        assert_eq!(endpoint, preface::Endpoint::ConsensusNet);
        tracing::info!("Expect the handshake to fail");
        let res = handshake::inbound(ctx, &setup.keys[1], rng.gen(), &mut stream).await;
        assert_matches!(res, Err(handshake::Error::GenesisMismatch));

        tracing::info!("Try to connect to a node with a mismatching genesis.");
        let mut stream =
            preface::connect(ctx, *cfgs[0].server_addr, preface::Endpoint::ConsensusNet)
                .await
                .context("preface::connect")?;
        let res = handshake::outbound(
            ctx,
            &setup.keys[1],
            rng.gen(),
            &mut stream,
            &setup.keys[0].public(),
        )
        .await;
        tracing::info!(
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
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        let mut nodes: Vec<_> = cfgs
            .iter()
            .enumerate()
            .map(|(i, cfg)| {
                let (node, runner) = testonly::Instance::new(cfg.clone(), store.clone());
                s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
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
        let (node0, runner) = testonly::Instance::new(cfgs[0].clone(), store.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node0")));
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
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        let mut nodes: Vec<_> = cfgs
            .iter()
            .enumerate()
            .map(|(i, cfg)| {
                let (node, runner) = testonly::Instance::new(cfg.clone(), store.clone());
                let i = ctx::NoCopy(i);
                s.spawn_bg(async {
                    let i = i;
                    runner
                        .run(ctx)
                        .instrument(tracing::info_span!("node", i = *i))
                        .await
                        .context(*i)
                });
                node
            })
            .collect();

        tracing::info!("waiting for all connections to be established");
        for n in &mut nodes {
            n.wait_for_consensus_connections().await;
        }
        for _ in 0..10 {
            let want: validator::Signed<validator::ConsensusMsg> = rng.gen();
            let in_message = io::ConsensusInputMessage {
                message: want.clone(),
                recipient: io::Target::Validator(
                    nodes[1].cfg().validator_key.as_ref().unwrap().public(),
                ),
            };
            nodes[0].pipe.send(in_message.into());

            let got = loop {
                let output_message = nodes[1].pipe.recv(ctx).await.unwrap();
                let io::OutputMessage::Consensus(got) = output_message else {
                    continue;
                };
                break got;
            };
            assert_eq!(want, got.msg);
        }
        Ok(())
    })
    .await
    .unwrap();
}
