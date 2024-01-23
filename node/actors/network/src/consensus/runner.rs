//! run_client routine maintaining outbound connections to validators.

use super::handshake;
use crate::{io, noise, preface, rpc, State};
use anyhow::Context as _;
use std::{collections::HashMap, sync::Arc};
use zksync_concurrency::{ctx, ctx::channel, oneshot, scope, sync, time};
use zksync_consensus_roles::validator;

/// How often we should retry to establish a connection to a validator.
/// TODO(gprusak): once it becomes relevant, choose a more appropriate retry strategy.
const CONNECT_RETRY: time::Duration = time::Duration::seconds(20);
/// A ping request is sent periodically. If ping response doesn't arrive
/// within PING_TIMEOUT, we close the connection.
const PING_TIMEOUT: time::Duration = time::Duration::seconds(10);
/// Each consensus message is expected to be delivered within MSG_TIMEOUT.
/// After that time the message is dropped.
/// TODO(gprusak): for liveness we should retry sending messages until the view
/// changes. That requires tighter integration with the consensus.
const MSG_TIMEOUT: time::Duration = time::Duration::seconds(10);

#[async_trait::async_trait]
impl rpc::Handler<rpc::consensus::Rpc> for channel::UnboundedSender<io::OutputMessage> {
    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        req: rpc::consensus::Req,
    ) -> anyhow::Result<rpc::consensus::Resp> {
        let (send, recv) = oneshot::channel();
        self.send(io::OutputMessage::Consensus(io::ConsensusReq {
            msg: req.0,
            ack: send,
        }));
        recv.recv_or_disconnected(ctx).await??;
        Ok(rpc::consensus::Resp)
    }
}

/// Performs handshake of an inbound stream.
/// Closes the stream if there is another inbound stream opened from the same validator.
pub(crate) async fn run_inbound_stream(
    ctx: &ctx::Ctx,
    state: &super::State,
    sender: &channel::UnboundedSender<io::OutputMessage>,
    mut stream: noise::Stream,
) -> anyhow::Result<()> {
    let peer = handshake::inbound(ctx, &state.cfg.key, &mut stream).await?;
    state.inbound.insert(peer.clone()).await?;
    let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx);
    let res = scope::run!(ctx, |ctx, s| async {
        s.spawn(ping_client.ping_loop(ctx, PING_TIMEOUT));
        rpc::Service::new()
            .add_client(&ping_client)
            .add_server(rpc::ping::Server)
            .add_server(sender.clone())
            .run(ctx, stream)
            .await?;
        Ok(())
    })
    .await;
    state.inbound.remove(&peer).await;
    res
}

async fn run_outbound_stream(
    ctx: &ctx::Ctx,
    state: &super::State,
    client: &rpc::Client<rpc::consensus::Rpc>,
    peer: &validator::PublicKey,
    addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    let mut stream = preface::connect(ctx, addr, preface::Endpoint::ConsensusNet).await?;
    handshake::outbound(ctx, &state.cfg.key, &mut stream, peer).await?;
    state.outbound.insert(peer.clone()).await?;
    let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx);
    let res = scope::run!(ctx, |ctx, s| async {
        s.spawn(ping_client.ping_loop(ctx, PING_TIMEOUT));
        rpc::Service::new()
            .add_client(&ping_client)
            .add_server(rpc::ping::Server)
            .add_client(client)
            .run(ctx, stream)
            .await?;
        Ok(())
    })
    .await;
    state.outbound.remove(peer).await;
    res
}

/// Runs an Rpc client trying to maintain 1 outbound connection per validator.
pub(crate) async fn run_client(
    ctx: &ctx::Ctx,
    state: &super::State,
    shared_state: &State,
    mut receiver: channel::UnboundedReceiver<io::ConsensusInputMessage>,
) -> anyhow::Result<()> {
    let clients: HashMap<_, _> = shared_state
        .cfg
        .validators
        .iter()
        .map(|peer| (peer.clone(), rpc::Client::<rpc::consensus::Rpc>::new(ctx)))
        .collect();
    scope::run!(ctx, |ctx, s| async {
        // Spawn outbound connections.
        for (peer, client) in &clients {
            s.spawn::<()>(async {
                let client = &*client;
                let addrs = &mut shared_state.gossip.validator_addrs.subscribe();
                let mut addr = None;
                while ctx.is_active() {
                    if let Ok(new) =
                        sync::wait_for(&ctx.with_timeout(CONNECT_RETRY), addrs, |addrs| {
                            addrs.get(peer).map(|x| x.msg.addr) != addr
                        })
                        .await
                    {
                        addr = new.get(peer).map(|x| x.msg.addr);
                    }
                    if let Some(addr) = addr {
                        if let Err(err) = run_outbound_stream(ctx, state, client, peer, addr).await
                        {
                            tracing::info!("run_outbound_stream({addr}): {err:#}");
                        }
                    }
                }
                Ok(())
            });
        }

        // Call RPCs.
        while let Ok(msg) = receiver.recv(ctx).await {
            match msg.recipient {
                io::Target::Validator(val) => {
                    let client = clients.get(&val).context("unknown validator")?;
                    s.spawn(async {
                        let req = rpc::consensus::Req(msg.message);
                        if let Err(err) = client.call(&ctx.with_timeout(MSG_TIMEOUT), &req).await {
                            tracing::info!("client.consensus(): {err:#}");
                        }
                        Ok(())
                    });
                }
                io::Target::Broadcast => {
                    let req = Arc::new(rpc::consensus::Req(msg.message));
                    for client in clients.values() {
                        let req = req.clone();
                        s.spawn(async {
                            let req = req;
                            if let Err(err) =
                                client.call(&ctx.with_timeout(MSG_TIMEOUT), &req).await
                            {
                                tracing::info!("client.consensus(): {err:#}");
                            }
                            Ok(())
                        });
                    }
                }
            }
        }
        Ok(())
    })
    .await
}
