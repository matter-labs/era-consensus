use super::{handshake, ValidatorAddrs};
use crate::{consensus, event::Event, io, noise, preface, rpc, State};
use async_trait::async_trait;
use std::sync::Arc;
use zksync_concurrency::{
    ctx::{self, channel},
    oneshot, scope, sync, time,
};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::BlockStore;
use zksync_protobuf::kB;

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

struct PushValidatorAddrsServer<'a>(&'a State);

#[async_trait]
impl rpc::Handler<rpc::push_validator_addrs::Rpc> for PushValidatorAddrsServer<'_> {
    fn max_req_size(&self) -> usize {
        100 * kB
    }
    async fn handle(
        &self,
        _ctx: &ctx::Ctx,
        req: rpc::push_validator_addrs::Req,
    ) -> anyhow::Result<()> {
        self.0.event(Event::ValidatorAddrsUpdated);
        self.0
            .gossip
            .validator_addrs
            .update(&self.0.cfg.validators, &req.0[..])
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct PushBlockStoreStateServer<'a> {
    peer: &'a node::PublicKey,
    sender: &'a channel::UnboundedSender<io::OutputMessage>,
}

#[async_trait]
impl rpc::Handler<rpc::push_block_store_state::Rpc> for PushBlockStoreStateServer<'_> {
    fn max_req_size(&self) -> usize {
        10 * kB
    }
    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        req: rpc::push_block_store_state::Req,
    ) -> anyhow::Result<()> {
        let (response, response_receiver) = oneshot::channel();
        let message = io::SyncBlocksRequest::UpdatePeerSyncState {
            peer: self.peer.clone(),
            state: req.0,
            response,
        };
        self.sender.send(message.into());
        response_receiver.recv_or_disconnected(ctx).await??;
        Ok(())
    }
}

#[async_trait]
impl rpc::Handler<rpc::get_block::Rpc> for &BlockStore {
    fn max_req_size(&self) -> usize {
        kB
    }
    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        req: rpc::get_block::Req,
    ) -> anyhow::Result<rpc::get_block::Resp> {
        Ok(rpc::get_block::Resp(self.block(ctx, req.0).await?))
    }
}

async fn run_stream(
    ctx: &ctx::Ctx,
    state: &State,
    peer: &node::PublicKey,
    sender: &channel::UnboundedSender<io::OutputMessage>,
    stream: noise::Stream,
) -> anyhow::Result<()> {
    let push_validator_addrs_client = rpc::Client::<rpc::push_validator_addrs::Rpc>::new(ctx);
    let push_validator_addrs_server = PushValidatorAddrsServer(state);
    let push_block_store_state_client = rpc::Client::<rpc::push_block_store_state::Rpc>::new(ctx);
    let push_block_store_state_server = PushBlockStoreStateServer { peer, sender };

    let get_block_client = Arc::new(rpc::Client::<rpc::get_block::Rpc>::new(ctx));
    state
        .gossip
        .get_block_clients
        .insert(peer.clone(), get_block_client.clone());

    let res = scope::run!(ctx, |ctx, s| async {
        let mut service = rpc::Service::new()
            .add_client(&push_validator_addrs_client)
            .add_server(push_validator_addrs_server)
            .add_client(&push_block_store_state_client)
            .add_server(push_block_store_state_server)
            .add_client(&get_block_client)
            .add_server(&*state.gossip.block_store)
            .add_server(rpc::ping::Server);

        if state.cfg.enable_pings {
            let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx);
            service = service.add_client(&ping_client);
            s.spawn(async {
                let ping_client = ping_client;
                ping_client.ping_loop(ctx, PING_TIMEOUT).await
            });
        }

        // Push block store state updates to peer.
        s.spawn::<()>(async {
            let mut sub = state.gossip.block_store.subscribe();
            sub.mark_changed();
            loop {
                let state = sync::changed(ctx, &mut sub).await?.clone();
                let req = rpc::push_block_store_state::Req(state);
                push_block_store_state_client.call(ctx, &req, kB).await?;
            }
        });

        s.spawn::<()>(async {
            // Push validator addrs updates to peer.
            let mut old = ValidatorAddrs::default();
            let mut sub = state.gossip.validator_addrs.subscribe();
            sub.mark_changed();
            loop {
                let new = sync::changed(ctx, &mut sub).await?.clone();
                let diff = new.get_newer(&old);
                if diff.is_empty() {
                    continue;
                }
                old = new;
                let req = rpc::push_validator_addrs::Req(diff);
                push_validator_addrs_client.call(ctx, &req, kB).await?;
            }
        });

        service.run(ctx, stream).await?;
        Ok(())
    })
    .await;

    state
        .gossip
        .get_block_clients
        .remove(peer.clone(), get_block_client);
    res
}

/// Handles an inbound stream.
/// Closes the stream if there is another inbound stream opened from the same peer.
pub(crate) async fn run_inbound_stream(
    ctx: &ctx::Ctx,
    state: &State,
    sender: &channel::UnboundedSender<io::OutputMessage>,
    mut stream: noise::Stream,
) -> anyhow::Result<()> {
    let peer = handshake::inbound(ctx, &state.gossip.cfg, &mut stream).await?;
    tracing::Span::current().record("peer", tracing::field::debug(&peer));
    state.gossip.inbound.insert(peer.clone()).await?;
    let res = run_stream(ctx, state, &peer, sender, stream).await;
    state.gossip.inbound.remove(&peer).await;
    res
}

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
    let res = run_stream(ctx, state, peer, sender, stream).await;
    state.gossip.outbound.remove(peer).await;
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
                    let message = message;
                    let io::SyncBlocksInputMessage::GetBlock {
                        recipient,
                        number,
                        response,
                    } = message;
                    let _ = response.send(
                        match state
                            .gossip
                            .get_block(ctx, &recipient, number, state.cfg.max_block_size)
                            .await
                        {
                            Ok(Some(block)) => Ok(block),
                            Ok(None) => Err(io::GetBlockError::NotAvailable),
                            Err(err) => Err(io::GetBlockError::Internal(err)),
                        },
                    );
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
