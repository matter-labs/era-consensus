use super::{handshake, ValidatorAddrs};
use crate::{
    consensus,
    event::{Event, StreamEvent},
    io, noise, preface, rpc, State,
};
use anyhow::Context;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::Instrument as _;
use zksync_concurrency::{
    ctx::{self, channel},
    oneshot, scope,
    sync::{self, watch},
    time,
};
use zksync_consensus_roles::{node, validator};

/// How often we should retry to establish a connection to a validator.
/// TODO(gprusak): once it becomes relevant, choose a more appropriate retry strategy.
pub(crate) const CONNECT_RETRY: time::Duration = time::Duration::seconds(20);
/// A ping request is sent periodically. If ping response doesn't arrive
/// within PING_TIMEOUT, we close the connection.
const PING_TIMEOUT: time::Duration = time::Duration::seconds(5);
/// Frequency at which the validator broadcasts its own IP address.
/// Although the IP is not likely to change while validator is running,
/// we do this periodically, so that the network can observe if validator
/// is down.
const ADDRESS_ANNOUNCER_INTERVAL: time::Duration = time::Duration::minutes(10);

struct ValidatorAddrsServer<'a> {
    state: &'a State,
    peer_validator_addrs: sync::Mutex<ValidatorAddrs>,
}

impl<'a> ValidatorAddrsServer<'a> {
    fn new(state: &'a State) -> Self {
        Self {
            state,
            peer_validator_addrs: sync::Mutex::default(),
        }
    }
}

#[async_trait]
impl rpc::Handler<rpc::sync_validator_addrs::Rpc> for ValidatorAddrsServer<'_> {
    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        _req: rpc::sync_validator_addrs::Req,
    ) -> anyhow::Result<rpc::sync_validator_addrs::Resp> {
        let mut old = sync::lock(ctx, &self.peer_validator_addrs)
            .await?
            .into_async();
        let mut sub = self.state.gossip.validator_addrs.subscribe();
        loop {
            let new = sync::changed(ctx, &mut sub).await?.clone();
            let diff = new.get_newer(&old);
            if diff.is_empty() {
                continue;
            }
            *old = new;
            return Ok(rpc::sync_validator_addrs::Resp(diff));
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SyncBlocksServer<'a> {
    peer: &'a node::PublicKey,
    sender: &'a channel::UnboundedSender<io::OutputMessage>,
}

#[async_trait]
impl rpc::Handler<rpc::sync_blocks::PushSyncStateRpc> for SyncBlocksServer<'_> {
    #[tracing::instrument(
        level = "trace",
        skip_all,
        err,
        fields(update = ?req.numbers()),
    )]
    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        req: io::SyncState,
    ) -> anyhow::Result<rpc::sync_blocks::SyncStateResponse> {
        let (response, response_receiver) = oneshot::channel();
        let message = io::SyncBlocksRequest::UpdatePeerSyncState {
            peer: self.peer.clone(),
            state: Box::new(req),
            response,
        };
        self.sender.send(message.into());
        response_receiver.recv_or_disconnected(ctx).await??;
        Ok(rpc::sync_blocks::SyncStateResponse)
    }
}

#[tracing::instrument(level = "trace", skip_all, err)]
async fn run_sync_blocks_client(
    ctx: &ctx::Ctx,
    client: &rpc::Client<rpc::sync_blocks::PushSyncStateRpc>,
    mut sync_state_subscriber: watch::Receiver<io::SyncState>,
) -> anyhow::Result<()> {
    // Send the initial value immediately.
    let mut updated_state = sync_state_subscriber.borrow_and_update().clone();
    loop {
        let span = tracing::trace_span!("push_sync_state", update = ?updated_state.numbers());
        let push_sync_state = async {
            tracing::trace!("sending");
            client
                .call(ctx, &updated_state)
                .await
                .context("sync_blocks_client")?;
            tracing::trace!("sent");
            updated_state = sync::changed(ctx, &mut sync_state_subscriber)
                .await
                .context("sync_blocks_client update")?
                .clone();
            anyhow::Ok(())
        };
        push_sync_state.instrument(span).await?;
    }
}

#[async_trait]
impl rpc::Handler<rpc::sync_blocks::GetBlockRpc> for SyncBlocksServer<'_> {
    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        req: rpc::sync_blocks::GetBlockRequest,
    ) -> anyhow::Result<rpc::sync_blocks::GetBlockResponse> {
        let (response, response_receiver) = oneshot::channel();
        let message = io::SyncBlocksRequest::GetBlock {
            block_number: req.0,
            response,
        };
        self.sender.send(message.into());
        let response = response_receiver.recv_or_disconnected(ctx).await??;
        Ok(response.into())
    }
}

async fn run_stream(
    ctx: &ctx::Ctx,
    state: &State,
    peer: &node::PublicKey,
    sender: &channel::UnboundedSender<io::OutputMessage>,
    get_blocks_client: &rpc::Client<rpc::sync_blocks::GetBlockRpc>,
    stream: noise::Stream,
) -> anyhow::Result<()> {
    let sync_validator_addrs_client = rpc::Client::<rpc::sync_validator_addrs::Rpc>::new(ctx);
    let sync_blocks_server = SyncBlocksServer { peer, sender };
    let sync_state_client = rpc::Client::<rpc::sync_blocks::PushSyncStateRpc>::new(ctx);

    let enable_pings = state.gossip.cfg.enable_pings;

    scope::run!(ctx, |ctx, s| async {
        s.spawn(async {
            let mut service = rpc::Service::new()
                .add_client(&sync_validator_addrs_client)
                .add_server(ValidatorAddrsServer::new(state))
                .add_client(&sync_state_client)
                .add_server::<rpc::sync_blocks::PushSyncStateRpc>(sync_blocks_server)
                .add_client(get_blocks_client)
                .add_server::<rpc::sync_blocks::GetBlockRpc>(sync_blocks_server)
                .add_server(rpc::ping::Server);

            if enable_pings {
                let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx);
                service = service.add_client(&ping_client);
                let ctx = &*ctx;
                s.spawn(async {
                    let ping_client = ping_client;
                    ping_client.ping_loop(ctx, PING_TIMEOUT).await
                });
            }

            service.run(ctx, stream).await?;
            Ok(())
        });

        if let Some(sync_state_subscriber) = state.gossip.sync_state.clone() {
            s.spawn(run_sync_blocks_client(
                ctx,
                &sync_state_client,
                sync_state_subscriber,
            ));
        }

        loop {
            let resp = sync_validator_addrs_client
                .call(ctx, &rpc::sync_validator_addrs::Req)
                .await?;
            state
                .gossip
                .validator_addrs
                .update(&state.cfg.validators, &resp.0[..])
                .await?;
            state.event(Event::ValidatorAddrsUpdated);
        }
    })
    .await
}

async fn handle_clients_and_run_stream(
    ctx: &ctx::Ctx,
    state: &State,
    peer: &node::PublicKey,
    sender: &channel::UnboundedSender<io::OutputMessage>,
    stream: noise::Stream,
) -> anyhow::Result<()> {
    let clients = &state.gossip.get_block_clients;
    let get_blocks_client = rpc::Client::<rpc::sync_blocks::GetBlockRpc>::new(ctx);
    let get_blocks_client = Arc::new(get_blocks_client);
    sync::lock(ctx, clients)
        .await
        .context("get_block_clients")?
        .insert(peer.clone(), get_blocks_client.clone());

    let res = run_stream(ctx, state, peer, sender, &get_blocks_client, stream).await;

    // We remove the peer client unconditionally, even if the context is cancelled, so that the state
    // is consistent.
    let mut client_map = clients.lock().await;
    // The get_blocks client might have been replaced because we might have both an inbound and outbound
    // connections to the same peer, and we utilize only one of them to send `get_block` requests.
    // Thus, we need to check it before removing from the map.
    let is_same_client = client_map
        .get(peer)
        .map_or(false, |client| Arc::ptr_eq(client, &get_blocks_client));
    if is_same_client {
        client_map.remove(peer);
    }
    res
}

/// Handles an inbound stream.
/// Closes the stream if there is another inbound stream opened from the same peer.
#[tracing::instrument(
    level = "trace",
    skip_all,
    err,
    fields(my_key = ?state.gossip.cfg.key.public(), peer),
)]
pub(crate) async fn run_inbound_stream(
    ctx: &ctx::Ctx,
    state: &State,
    sender: &channel::UnboundedSender<io::OutputMessage>,
    mut stream: noise::Stream,
) -> anyhow::Result<()> {
    let peer = handshake::inbound(ctx, &state.gossip.cfg, &mut stream).await?;
    tracing::Span::current().record("peer", tracing::field::debug(&peer));

    state.gossip.inbound.insert(peer.clone()).await?;
    state.event(Event::Gossip(StreamEvent::InboundOpened(peer.clone())));

    let res = handle_clients_and_run_stream(ctx, state, &peer, sender, stream).await;

    state.gossip.inbound.remove(&peer).await;
    state.event(Event::Gossip(StreamEvent::InboundClosed(peer)));

    res
}

#[tracing::instrument(
    level = "trace",
    skip(ctx, state, sender),
    err,
    fields(my_key = ?state.gossip.cfg.key.public())
)]
async fn run_outbound_stream(
    ctx: &ctx::Ctx,
    state: &State,
    sender: &channel::UnboundedSender<io::OutputMessage>,
    peer: &node::PublicKey,
    addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    let mut stream = preface::connect(ctx, addr, preface::Endpoint::GossipNet).await?;
    handshake::outbound(ctx, &state.gossip.cfg, &mut stream, peer).await?;

    state.gossip.outbound.insert(peer.clone()).await?;
    state.event(Event::Gossip(StreamEvent::OutboundOpened(peer.clone())));

    let res = handle_clients_and_run_stream(ctx, state, peer, sender, stream).await;

    state.gossip.outbound.remove(peer).await;
    state.event(Event::Gossip(StreamEvent::OutboundClosed(peer.clone())));

    res
}

async fn run_address_announcer(
    ctx: &ctx::Ctx,
    state: &State,
    consensus_state: &consensus::State,
) -> ctx::OrCanceled<()> {
    let key = &consensus_state.cfg.key;
    let my_addr = consensus_state.cfg.public_addr;
    let mut sub = state.gossip.validator_addrs.subscribe();
    loop {
        if !ctx.is_active() {
            return Err(ctx::Canceled);
        }
        let ctx = &ctx.with_timeout(ADDRESS_ANNOUNCER_INTERVAL);
        let _ = sync::wait_for(ctx, &mut sub, |got| {
            got.get(&key.public()).map(|x| &x.msg.addr) != Some(&my_addr)
        })
        .await;
        let next_version = sub
            .borrow()
            .get(&key.public())
            .map(|x| x.msg.version + 1)
            .unwrap_or(0);
        state
            .gossip
            .validator_addrs
            .update(
                &state.cfg.validators,
                &[Arc::new(key.sign_msg(validator::NetAddress {
                    addr: my_addr,
                    version: next_version,
                    timestamp: ctx.now_utc(),
                }))],
            )
            .await
            .unwrap();
    }
}

#[tracing::instrument(level = "trace", skip_all, err, fields(recipient, number))]
async fn handle_sync_blocks_message(
    ctx: &ctx::Ctx,
    state: &State,
    message: io::SyncBlocksInputMessage,
) -> anyhow::Result<()> {
    let io::SyncBlocksInputMessage::GetBlock {
        recipient,
        number,
        response,
    } = message;

    tracing::Span::current()
        .record("recipient", tracing::field::debug(&recipient))
        .record("number", number.0);

    let clients = &state.gossip.get_block_clients;
    let recipient_client = sync::lock(ctx, clients).await?.get(&recipient).cloned();
    let recipient_client = recipient_client.context("recipient is unreachable")?;
    let request = rpc::sync_blocks::GetBlockRequest(number);
    let peer_response = recipient_client.call(ctx, &request).await?.0;

    response
        .send(peer_response)
        .map_err(|_| anyhow::anyhow!("cannot send response to request to request initiator"))
}

/// Runs an RPC client trying to maintain 1 outbound connection per validator.
pub(crate) async fn run_client(
    ctx: &ctx::Ctx,
    state: &State,
    sender: &channel::UnboundedSender<io::OutputMessage>,
    mut receiver: channel::UnboundedReceiver<io::SyncBlocksInputMessage>,
) -> anyhow::Result<()> {
    scope::run!(ctx, |ctx, s| async {
        // Spawn a tasks handling static outbound connections.
        for (peer, addr) in &state.gossip.cfg.static_outbound {
            s.spawn::<()>(async {
                loop {
                    let run_result = run_outbound_stream(ctx, state, sender, peer, *addr).await;
                    if let Err(err) = run_result {
                        tracing::info!("run_client_stream(): {err:#}");
                    }
                    ctx.sleep(CONNECT_RETRY).await?;
                }
            });
        }

        s.spawn(async {
            while let Ok(message) = receiver.recv(ctx).await {
                s.spawn(async {
                    handle_sync_blocks_message(ctx, state, message).await.ok();
                    // ^ The client errors are logged by the method instrumentation wrapper,
                    // so we don't need logging here.
                    Ok(())
                });
            }
            Ok(())
        });

        if let Some(consensus_state) = &state.consensus {
            run_address_announcer(ctx, state, consensus_state).await
        } else {
            Ok(())
        }
    })
    .await
    .ok();

    Ok(())
}
