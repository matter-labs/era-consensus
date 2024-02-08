//! Consensus network is a full graph of connections between all validators.
//! BFT consensus messages are exchanged over this network.
use crate::pool::PoolWatch;
use crate::gossip;
use crate::config;
use std::sync::Arc;
use std::collections::{HashSet,HashMap};
use zksync_consensus_roles::{validator};
use zksync_concurrency::{ctx, oneshot, scope, sync, time};
use zksync_protobuf::kB;
use crate::{io, noise, preface, rpc};
use anyhow::Context as _;

mod handshake;
#[cfg(test)]
mod tests;


const RESP_MAX_SIZE : usize = kB;
/// Frequency at which the validator broadcasts its own IP address.
/// Although the IP is not likely to change while validator is running,
/// we do this periodically, so that the network can observe if validator
/// is down.
const ADDRESS_ANNOUNCER_INTERVAL: time::Duration = time::Duration::minutes(10);

/// Consensus network state.
pub(crate) struct Network {
    pub(crate) gossip: Arc<gossip::Network>,
    pub(crate) key: validator::SecretKey, 
    /// Set of the currently open inbound connections.
    pub(crate) inbound: PoolWatch<validator::PublicKey>,
    /// Set of the currently open outbound connections.
    pub(crate) outbound: PoolWatch<validator::PublicKey>,

    pub(crate) clients: HashMap<validator::PublicKey, rpc::Client::<rpc::consensus::Rpc>>,
}

#[async_trait::async_trait]
impl rpc::Handler<rpc::consensus::Rpc> for &Network {
    /// Here we bound the buffering of incoming consensus messages.
    fn max_req_size(&self) -> usize {
        self.gossip.cfg.max_block_size.saturating_add(kB)
    }

    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        req: rpc::consensus::Req,
    ) -> anyhow::Result<rpc::consensus::Resp> {
        let (send, recv) = oneshot::channel();
        self.gossip.sender
            .send(io::OutputMessage::Consensus(io::ConsensusReq {
                msg: req.0,
                ack: send,
            }));
        recv.recv_or_disconnected(ctx).await??;
        Ok(rpc::consensus::Resp)
    }
}

impl Network {
    /// Constructs a new consensus network state.
    pub(crate) fn new(ctx: &ctx::Ctx, gossip: Arc<gossip::Network>) -> Option<Arc<Self>> {
        let key = gossip.cfg.validator_key.clone()?;
        let validators: HashSet<_> = gossip.cfg.genesis.validators.iter().cloned().collect();
        Some(Arc::new(Self {
            gossip,
            key,
            inbound: PoolWatch::new(validators.clone(), 0),
            outbound: PoolWatch::new(validators.clone(), 0),
            clients: validators.iter().map(|peer| (peer.clone(), rpc::Client::new(ctx)))
                .collect(),
        }))
    }

    pub(crate) async fn broadcast(&self, ctx: &ctx::Ctx, msg: validator::Signed<validator::ConsensusMsg>) -> anyhow::Result<()> {
        let req = rpc::consensus::Req(msg);
        scope::run!(ctx, |ctx,s| async {
            for (peer,client) in &self.clients {
                s.spawn(async {
                    if let Err(err) = client.call(ctx, &req, RESP_MAX_SIZE).await {
                        tracing::info!("send({:?},<ConsensusMsg>): {err:#}",&*peer);
                    }
                    Ok(())
                });
            }
            Ok(())
        }).await
    }

    pub(crate) async fn send(&self, ctx: &ctx::Ctx, key: &validator::PublicKey, msg: validator::Signed<validator::ConsensusMsg>) -> anyhow::Result<()> {   
        let client = self.clients.get(key).context("not an active validator")?;
        client.call(ctx, &rpc::consensus::Req(msg), RESP_MAX_SIZE).await?;
        Ok(())
    }

    /// Performs handshake of an inbound stream.
    /// Closes the stream if there is another inbound stream opened from the same validator.
    pub(crate) async fn run_inbound_stream(
        &self,
        ctx: &ctx::Ctx,
        mut stream: noise::Stream,
    ) -> anyhow::Result<()> {
        let peer = handshake::inbound(ctx, &self.key, self.gossip.cfg.genesis.hash(), &mut stream).await?;
        self.inbound.insert(peer.clone()).await?;
        let res = scope::run!(ctx, |ctx, s| async {
            let mut service = rpc::Service::new()
                .add_server(rpc::ping::Server)
                .add_server(self);
            if let Some(ping_timeout) = &self.gossip.cfg.ping_timeout {
                let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx);
                service = service.add_client(&ping_client);
                s.spawn(async {
                    let ping_client = ping_client;
                    ping_client.ping_loop(ctx, *ping_timeout).await
                });
            }
            service.run(ctx, stream).await?;
            Ok(())
        })
        .await;
        self.inbound.remove(&peer).await;
        res
    }

    async fn run_outbound_stream(
        &self,
        ctx: &ctx::Ctx,
        peer: &validator::PublicKey,
        addr: std::net::SocketAddr,
    ) -> anyhow::Result<()> {
        let client = self.clients.get(peer).context("not an active validator")?; 
        let mut stream = preface::connect(ctx, addr, preface::Endpoint::ConsensusNet).await?;
        handshake::outbound(ctx, &self.key, self.gossip.cfg.genesis.hash(), &mut stream, peer).await?;
        self.outbound.insert(peer.clone()).await?;
        let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx);
        let res = scope::run!(ctx, |ctx, s| async {
            let mut service = rpc::Service::new()
                .add_client(&ping_client)
                .add_server(rpc::ping::Server)
                .add_client(client);
            if let Some(ping_timeout) = &self.gossip.cfg.ping_timeout {
                let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx);
                service = service.add_client(&ping_client);
                s.spawn(async {
                    let ping_client = ping_client;
                    ping_client.ping_loop(ctx, *ping_timeout).await
                });
            }
            service.run(ctx, stream).await?;
            Ok(())
        })
        .await;
        self.outbound.remove(peer).await;
        res
    }

    pub(crate) async fn maintain_connection(&self, ctx: &ctx::Ctx, peer: &validator::PublicKey) -> anyhow::Result<()> {
        let addrs = &mut self.gossip.validator_addrs.subscribe();
        let mut addr = None;
        while ctx.is_active() {
            // Wait for a new address, or retry with the old one after timeout.
            if let Ok(new) =
                sync::wait_for(&ctx.with_timeout(config::CONNECT_RETRY), addrs, |addrs| {
                    addrs.get(peer).map(|x| x.msg.addr) != addr
                })
                .await
            {
                addr = new.get(peer).map(|x| x.msg.addr);
            }
            let Some(addr) = addr else { continue };
            if let Err(err) = self.run_outbound_stream(ctx, peer, addr).await {
                tracing::info!("run_outbound_stream({peer:?},{addr}): {err:#}");
            }
        }
        Ok(())
    }

    pub(crate) async fn run_address_announcer(&self, ctx: &ctx::Ctx) {
        let my_addr = self.gossip.cfg.public_addr;
        let mut sub = self.gossip.validator_addrs.subscribe();
        while ctx.is_active() {
            let ctx = &ctx.with_timeout(ADDRESS_ANNOUNCER_INTERVAL);
            let _ = sync::wait_for(ctx, &mut sub, |got| {
                got.get(&self.key.public()).map(|x| &x.msg.addr) != Some(&my_addr)
            })
            .await;
            let next_version = sub
                .borrow()
                .get(&self.key.public())
                .map(|x| x.msg.version + 1)
                .unwrap_or(0);
            self
                .gossip
                .validator_addrs
                .update(
                    &self.gossip.cfg.genesis.validators,
                    &[Arc::new(self.key.sign_msg(validator::NetAddress {
                        addr: my_addr,
                        version: next_version,
                        timestamp: ctx.now_utc(),
                    }))],
                )
                .await
                .unwrap();
        }
    }
}
