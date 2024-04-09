//! Consensus network is a full graph of connections between all validators.
//! BFT consensus messages are exchanged over this network.
use crate::rpc::Rpc as _;
use crate::{config, gossip, io, noise, pool::PoolWatch, preface, rpc};
use anyhow::Context as _;
use rand::seq::SliceRandom;
use std::{collections::HashSet, sync::Arc};
use tracing::Instrument as _;
use zksync_concurrency::{ctx, oneshot, scope, sync, time};
use zksync_consensus_roles::validator;
use zksync_protobuf::kB;

mod handshake;
#[cfg(test)]
mod tests;

const RESP_MAX_SIZE: usize = kB;
/// Frequency at which the validator broadcasts its own IP address.
/// Although the IP is not likely to change while validator is running,
/// we do this periodically, so that the network can observe if validator
/// is down.
const ADDRESS_ANNOUNCER_INTERVAL: time::Duration = time::Duration::minutes(10);

/// Outbound connection state.
pub(crate) struct Connection {
    /// Peer's address.
    /// This is not used for now, but will be required for the debug page.
    #[allow(dead_code)]
    addr: std::net::SocketAddr,
    consensus: rpc::Client<rpc::consensus::Rpc>,
}

/// Consensus network state.
pub(crate) struct Network {
    /// Gossip network state to bootstrap consensus network from.
    pub(crate) gossip: Arc<gossip::Network>,
    /// This validator's secret key.
    pub(crate) key: validator::SecretKey,
    /// Set of the currently open inbound connections.
    pub(crate) inbound: PoolWatch<validator::PublicKey, ()>,
    /// Set of the currently open outbound connections.
    pub(crate) outbound: PoolWatch<validator::PublicKey, Arc<Connection>>,
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
        self.gossip
            .sender
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
    pub(crate) fn new(gossip: Arc<gossip::Network>) -> Option<Arc<Self>> {
        let key = gossip.cfg.validator_key.clone()?;
        let validators: HashSet<_> = gossip.genesis().validators.iter_keys().cloned().collect();
        Some(Arc::new(Self {
            key,
            inbound: PoolWatch::new(validators.clone(), 0),
            outbound: PoolWatch::new(validators.clone(), 0),
            gossip,
        }))
    }

    /// Sends a message to all validators.
    pub(crate) async fn broadcast(
        &self,
        ctx: &ctx::Ctx,
        msg: validator::Signed<validator::ConsensusMsg>,
    ) -> anyhow::Result<()> {
        let req = rpc::consensus::Req(msg);
        let outbound = self.outbound.current();
        scope::run!(ctx, |ctx, s| async {
            for (peer, conn) in &outbound {
                s.spawn(async {
                    if let Err(err) = conn.consensus.call(ctx, &req, RESP_MAX_SIZE).await {
                        tracing::info!(
                            "send({:?},{}): {err:#}",
                            &*peer,
                            rpc::consensus::Rpc::submethod(&req)
                        );
                    }
                    Ok(())
                });
            }
            Ok(())
        })
        .await
    }

    /// Sends a message to the given validator.
    pub(crate) async fn send(
        &self,
        ctx: &ctx::Ctx,
        key: &validator::PublicKey,
        msg: validator::Signed<validator::ConsensusMsg>,
    ) -> anyhow::Result<()> {
        let outbound = self.outbound.current();
        let req = rpc::consensus::Req(msg);
        outbound
            .get(key)
            .context("not reachable")?
            .consensus
            .call(ctx, &req, RESP_MAX_SIZE)
            .await
            .with_context(|| rpc::consensus::Rpc::submethod(&req))?;
        Ok(())
    }

    /// Performs handshake of an inbound stream.
    /// Closes the stream if there is another inbound stream opened from the same validator.
    pub(crate) async fn run_inbound_stream(
        &self,
        ctx: &ctx::Ctx,
        mut stream: noise::Stream,
    ) -> anyhow::Result<()> {
        let peer =
            handshake::inbound(ctx, &self.key, self.gossip.genesis().hash(), &mut stream).await?;
        self.inbound.insert(peer.clone(), ()).await?;
        tracing::info!("inbound connection from {peer:?}");
        let res = scope::run!(ctx, |ctx, s| async {
            let mut service = rpc::Service::new()
                .add_server(rpc::ping::Server, rpc::ping::RATE)
                .add_server(self, self.gossip.cfg.rpc.consensus_rate);
            if let Some(ping_timeout) = &self.gossip.cfg.ping_timeout {
                let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx, rpc::ping::RATE);
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
        let mut stream = preface::connect(ctx, addr, preface::Endpoint::ConsensusNet).await?;
        handshake::outbound(
            ctx,
            &self.key,
            self.gossip.genesis().hash(),
            &mut stream,
            peer,
        )
        .await?;
        let conn = Arc::new(Connection {
            addr,
            consensus: rpc::Client::new(ctx, self.gossip.cfg.rpc.consensus_rate),
        });
        self.outbound.insert(peer.clone(), conn.clone()).await?;
        tracing::info!("outbound connection to {peer:?}");
        let res = scope::run!(ctx, |ctx, s| async {
            let mut service = rpc::Service::new()
                .add_server(rpc::ping::Server, rpc::ping::RATE)
                .add_client(&conn.consensus);
            if let Some(ping_timeout) = &self.gossip.cfg.ping_timeout {
                let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx, rpc::ping::RATE);
                service = service.add_client(&ping_client);
                s.spawn(async {
                    let ping_client = ping_client;
                    ping_client.ping_loop(ctx, *ping_timeout).await
                });
            }
            // If this is a loopback connection, announce periodically the address of this
            // validator to the network.
            // Note that this is executed only for outbound end of the loopback connection.
            // Inbound end doesn't know the public address of itself.
            if peer == &self.key.public() {
                s.spawn(async {
                    let mut sub = self.gossip.validator_addrs.subscribe();
                    while ctx.is_active() {
                        self.gossip
                            .validator_addrs
                            .announce(&self.key, addr, ctx.now_utc())
                            .await;
                        let _ = sync::wait_for(
                            &ctx.with_timeout(ADDRESS_ANNOUNCER_INTERVAL),
                            &mut sub,
                            |got| got.get(peer).map(|x| x.msg.addr) != Some(addr),
                        )
                        .await;
                    }
                    Ok(())
                });
            }
            service.run(ctx, stream).await?;
            Ok(())
        })
        .await;
        self.outbound.remove(peer).await;
        res
    }

    async fn run_loopback_stream(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let addr = *self
            .gossip
            .cfg
            .public_addr
            .resolve(ctx)
            .await?
            .context("resolve()")?
            .choose(&mut ctx.rng())
            .with_context(|| {
                format!("{:?} resolved to no addresses", self.gossip.cfg.public_addr)
            })?;
        self.run_outbound_stream(ctx, &self.key.public(), addr)
            .instrument(tracing::info_span!("{addr}"))
            .await
    }

    /// Maintains a connection to the given validator.
    /// If connection breaks, it tries to reconnect periodically.
    pub(crate) async fn maintain_connection(&self, ctx: &ctx::Ctx, peer: &validator::PublicKey) {
        // Loopback connection is a special case, because the address is taken from
        // the config rather than from `gossip.validator_addrs`.
        if &self.key.public() == peer {
            while ctx.is_active() {
                if let Err(err) = self.run_loopback_stream(ctx).await {
                    tracing::info!("run_loopback_stream(): {err:#}");
                }
            }
            return;
        }
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
            if let Err(err) = self
                .run_outbound_stream(ctx, peer, addr)
                .instrument(tracing::info_span!("{addr}"))
                .await
            {
                tracing::info!("run_outbound_stream({peer:?},{addr}): {err:#}");
            }
        }
    }
}
