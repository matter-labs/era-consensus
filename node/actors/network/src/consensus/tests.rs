use super::*;
use crate::{io, preface, rpc, run_network, testonly};
use anyhow::Context as _;
use rand::Rng;
use tracing::Instrument as _;
use zksync_concurrency::testonly::abort_on_panic;
use zksync_concurrency::{ctx, net, scope};
use zksync_consensus_roles::validator;
use zksync_consensus_utils::pipe;

#[tokio::test]
async fn test_one_connection_per_validator() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let mut nodes = testonly::Instance::new(rng, 3, 1);

    scope::run!(ctx, |ctx,s| async {
        for (i,n) in nodes.iter().enumerate() {
            let (network_pipe, _) = pipe::new();

            s.spawn_bg(run_network(
                ctx,
                n.state.clone(),
                network_pipe
            ).instrument(tracing::info_span!("node", i)));
        }

        tracing::info!("waiting for all gossip to be established");
        for node in &mut nodes {
            node.wait_for_gossip_connections().await;
        }

        tracing::info!("waiting for all connections to be established");
        for node in &mut nodes {
            node.wait_for_consensus_connections().await;
        }

        tracing::info!("Impersonate node 1, and try to establish additional connection to node 0. It should close automatically after the handshake.");
        let mut stream = preface::connect(
            ctx,
            *nodes[0].state.cfg.server_addr,
            preface::Endpoint::ConsensusNet,
        )
        .await?;

        handshake::outbound(
            ctx,
            &nodes[1].consensus_config().key,
            &mut stream,
            &nodes[0].consensus_config().key.public(),
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

#[tokio::test(flavor = "multi_thread")]
async fn test_address_change() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.));
    let rng = &mut ctx.rng();

    let mut nodes = testonly::Instance::new(rng, 5, 1);
    scope::run!(ctx, |ctx, s| async {
        for (i, n) in nodes.iter().enumerate().skip(1) {
            let (pipe, _) = pipe::new();
            s.spawn_bg(
                run_network(ctx, n.state.clone(), pipe).instrument(tracing::info_span!("node", i)),
            );
        }

        scope::run!(ctx, |ctx, s| async {
            let (pipe, _) = pipe::new();
            s.spawn_bg(
                run_network(ctx, nodes[0].state.clone(), pipe)
                    .instrument(tracing::info_span!("node0")),
            );
            for n in &nodes {
                n.wait_for_consensus_connections().await;
            }
            Ok(())
        })
        .await
        .context("run1")?;

        // All nodes should lose connection to node[0].
        let key0 = nodes[0].consensus_config().key.public();
        for node in &nodes {
            let consensus_state = node.state.consensus.as_ref().unwrap();
            consensus_state
                .inbound
                .subscribe()
                .wait_for(|got| !got.current().contains(&key0))
                .await
                .unwrap();
            consensus_state
                .outbound
                .subscribe()
                .wait_for(|got| !got.current().contains(&key0))
                .await
                .unwrap();
        }

        // Change the address of node[0].
        // Gossip network peers of node[0] won't be able to connect to it.
        // node[0] is expected to connect to its gossip peers.
        // Then it should broadcast its new address and the consensus network
        // should get reconstructed.
        let mut cfg = nodes[0].to_config();
        cfg.server_addr = net::tcp::testonly::reserve_listener();
        cfg.consensus.as_mut().unwrap().public_addr = *cfg.server_addr;
        nodes[0] = testonly::Instance::from_cfg(cfg);

        scope::run!(ctx, |ctx, s| async {
            let (pipe, _) = pipe::new();
            s.spawn_bg(
                run_network(ctx, nodes[0].state.clone(), pipe)
                    .instrument(tracing::info_span!("node0")),
            );
            for n in &nodes {
                n.wait_for_consensus_connections().await;
            }
            Ok(())
        })
        .await
        .context("run2")?;
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

    let mut nodes = testonly::Instance::new(rng, 2, 1);

    scope::run!(ctx, |ctx, s| async {
        let mut pipes = vec![];
        for (i, n) in nodes.iter().enumerate() {
            let (network_pipe, pipe) = pipe::new();
            pipes.push(pipe);
            s.spawn_bg(
                run_network(ctx, n.state.clone(), network_pipe)
                    .instrument(tracing::info_span!("node", i)),
            );
        }
        tracing::info!("waiting for all connections to be established");
        for n in &mut nodes {
            n.wait_for_consensus_connections().await;
        }
        for _ in 0..10 {
            let want: validator::Signed<validator::ConsensusMsg> = rng.gen();
            let in_message = io::ConsensusInputMessage {
                message: want.clone(),
                recipient: io::Target::Validator(nodes[1].consensus_config().key.public()),
            };
            pipes[0].send(in_message.into());

            let got = loop {
                let output_message = pipes[1].recv(ctx).await.unwrap();
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
