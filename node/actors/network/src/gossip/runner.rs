use super::{handshake, Network, ValidatorAddrs};
use crate::{io, noise, preface, rpc};
use async_trait::async_trait;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use zksync_concurrency::{ctx, oneshot, scope, sync};
use zksync_consensus_roles::node;
use zksync_consensus_storage::BlockStore;
use zksync_protobuf::kB;

struct PushValidatorAddrsServer<'a>(&'a Network);

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
        self.0
            .push_validator_addrs_calls
            .fetch_add(1, Ordering::SeqCst);
        self.0
            .validator_addrs
            .update(&self.0.genesis().validators, &req.0[..])
            .await?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct PushBlockStoreStateServer<'a> {
    peer: &'a node::PublicKey,
    net: &'a Network,
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
        self.net.sender.send(message.into());
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

impl Network {
    async fn run_stream(
        &self,
        ctx: &ctx::Ctx,
        peer: &node::PublicKey,
        stream: noise::Stream,
    ) -> anyhow::Result<()> {
        let push_validator_addrs_client = rpc::Client::<rpc::push_validator_addrs::Rpc>::new(
            ctx,
            self.cfg.rpc.push_validator_addrs_rate,
        );
        let push_validator_addrs_server = PushValidatorAddrsServer(self);
        let push_block_store_state_client = rpc::Client::<rpc::push_block_store_state::Rpc>::new(
            ctx,
            self.cfg.rpc.push_block_store_state_rate,
        );
        let push_block_store_state_server = PushBlockStoreStateServer { peer, net: self };

        let get_block_client = Arc::new(rpc::Client::<rpc::get_block::Rpc>::new(
            ctx,
            self.cfg.rpc.get_block_rate,
        ));
        self.get_block_clients
            .insert(peer.clone(), get_block_client.clone());

        let res = scope::run!(ctx, |ctx, s| async {
            let mut service = rpc::Service::new()
                .add_client(&push_validator_addrs_client)
                .add_server(
                    push_validator_addrs_server,
                    self.cfg.rpc.push_validator_addrs_rate,
                )
                .add_client(&push_block_store_state_client)
                .add_server(
                    push_block_store_state_server,
                    self.cfg.rpc.push_block_store_state_rate,
                )
                .add_client(&get_block_client)
                .add_server(&*self.block_store, self.cfg.rpc.get_block_rate)
                .add_server(rpc::ping::Server, rpc::ping::RATE);

            if let Some(ping_timeout) = &self.cfg.ping_timeout {
                let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx, rpc::ping::RATE);
                service = service.add_client(&ping_client);
                s.spawn(async {
                    let ping_client = ping_client;
                    ping_client.ping_loop(ctx, *ping_timeout).await
                });
            }

            // Push block store state updates to peer.
            s.spawn::<()>(async {
                let mut sub = self.block_store.subscribe();
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
                let mut sub = self.validator_addrs.subscribe();
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

        self.get_block_clients
            .remove(peer.clone(), get_block_client);
        res
    }

    /// Handles an inbound stream.
    /// Closes the stream if there is another inbound stream opened from the same peer.
    pub(crate) async fn run_inbound_stream(
        &self,
        ctx: &ctx::Ctx,
        mut stream: noise::Stream,
    ) -> anyhow::Result<()> {
        let peer =
            handshake::inbound(ctx, &self.cfg.gossip, self.genesis().hash(), &mut stream).await?;
        tracing::Span::current().record("peer", tracing::field::debug(&peer));
        self.inbound.insert(peer.clone()).await?;
        let res = self.run_stream(ctx, &peer, stream).await;
        self.inbound.remove(&peer).await;
        res
    }

    pub(crate) async fn run_outbound_stream(
        &self,
        ctx: &ctx::Ctx,
        peer: &node::PublicKey,
        addr: std::net::SocketAddr,
    ) -> anyhow::Result<()> {
        let mut stream = preface::connect(ctx, addr, preface::Endpoint::GossipNet).await?;
        handshake::outbound(
            ctx,
            &self.cfg.gossip,
            self.genesis().hash(),
            &mut stream,
            peer,
        )
        .await?;

        self.outbound.insert(peer.clone()).await?;
        let res = self.run_stream(ctx, peer, stream).await;
        self.outbound.remove(peer).await;
        res
    }
}
