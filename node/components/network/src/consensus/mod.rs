//! Consensus network is a full graph of connections between all validators.
//! BFT consensus messages are exchanged over this network.
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use anyhow::Context as _;
use rand::seq::SliceRandom;
use tracing::Instrument as _;
use zksync_concurrency::{ctx, oneshot, scope, sync, time};
use zksync_consensus_roles::validator;
use zksync_protobuf::kB;

use crate::{config, gossip, io, noise, pool::PoolWatch, preface, rpc, MeteredStreamStats};

mod handshake;
#[cfg(test)]
mod tests;

const RESP_MAX_SIZE: usize = kB;
/// Frequency at which the validator broadcasts its own IP address.
/// Although the IP is not likely to change while validator is running,
/// we do this periodically, so that the network can observe if validator
/// is down.
const ADDRESS_ANNOUNCER_INTERVAL: time::Duration = time::Duration::minutes(10);

type MsgPoolInner = BTreeMap<usize, Arc<io::ConsensusInputMessage>>;

/// Pool of messages to send.
/// Stored messages are available for retransmission in case of reconnect.
pub(crate) struct MsgPool(sync::watch::Sender<MsgPoolInner>);

/// Subscriber of the `MsgPool`. It allows to await messages addressed to
/// a specific peer.
pub(crate) struct MsgPoolRecv {
    recv: sync::watch::Receiver<MsgPoolInner>,
    next: usize,
}

impl MsgPool {
    /// Constructs an empty `MsgPool`.
    pub(crate) fn new() -> Self {
        Self(sync::watch::channel(BTreeMap::new()).0)
    }

    /// Inserts a message to the pool.
    pub(crate) fn send(&self, msg: Arc<io::ConsensusInputMessage>) {
        self.0.send_modify(|msgs| {
            // Select a unique ID for the new message: using `last ID+1` is ok (will NOT cause
            // an ID to be reused), because whenever we remove a message, we also insert a message.
            let next = msgs.last_key_value().map_or(0, |(k, _)| k + 1);

            msgs.insert(next, msg);
        });
    }

    /// Subscribes to messages in the pool directed to `target`.
    pub(crate) fn subscribe(&self) -> MsgPoolRecv {
        MsgPoolRecv {
            recv: self.0.subscribe(),
            next: 0,
        }
    }
}

impl MsgPoolRecv {
    /// Awaits the next message.
    pub(crate) async fn recv(
        &mut self,
        ctx: &ctx::Ctx,
    ) -> ctx::OrCanceled<Arc<io::ConsensusInputMessage>> {
        loop {
            if let Some((k, v)) = self.recv.borrow().range(self.next..).next() {
                self.next = k + 1;
                return Ok(v.clone());
            }
            sync::changed(ctx, &mut self.recv).await?;
        }
    }
}

/// Consensus network state.
pub(crate) struct Network {
    /// Gossip network state to bootstrap consensus network from.
    pub(crate) gossip: Arc<gossip::Network>,
    /// This validator's secret key.
    pub(crate) key: validator::SecretKey,
    /// Set of the currently open inbound connections.
    pub(crate) inbound: PoolWatch<validator::PublicKey, Arc<MeteredStreamStats>>,
    /// Set of the currently open outbound connections.
    pub(crate) outbound: PoolWatch<validator::PublicKey, Arc<MeteredStreamStats>>,
    /// Messages to be sent to validators.
    pub(crate) msg_pool: MsgPool,
}

#[async_trait::async_trait]
impl rpc::Handler<rpc::consensus::Rpc> for &Network {
    /// Here we bound the buffering of incoming consensus messages.
    fn max_req_size(&self) -> usize {
        self.gossip.cfg.max_block_size.saturating_add(100 * kB)
    }

    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        req: rpc::consensus::Req,
    ) -> anyhow::Result<rpc::consensus::Resp> {
        let (send, recv) = oneshot::channel();
        self.gossip.consensus_sender.send(io::ConsensusReq {
            msg: req.0,
            ack: send,
        });
        // TODO(gprusak): disconnection means that there message was rejected OR
        // that bft component is missing (in tests), which leads to unnecessary disconnects.
        let _ = recv.recv_or_disconnected(ctx).await?;
        Ok(rpc::consensus::Resp)
    }
}

impl Network {
    /// Constructs a new consensus network state.
    pub(crate) fn new(gossip: Arc<gossip::Network>) -> anyhow::Result<Option<Arc<Self>>> {
        // Check that we have both an epoch number and a validator key.
        // Otherwise, we can't have a consensus network.
        let Some(epoch_number) = gossip.epoch_number else {
            return Ok(None);
        };
        let Some(key) = gossip.cfg.validator_key.clone() else {
            return Ok(None);
        };

        let validators: HashSet<_> = gossip
            .validator_schedule()?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "network component has no validator schedule for epoch {}",
                    epoch_number
                )
            })?
            .keys()
            .cloned()
            .collect();

        Ok(Some(Arc::new(Self {
            key,
            inbound: PoolWatch::new(validators.clone(), 0),
            outbound: PoolWatch::new(validators.clone(), 0),
            gossip,
            msg_pool: MsgPool::new(),
        })))
    }

    /// Performs handshake of an inbound stream.
    /// Closes the stream if there is another inbound stream opened from the same validator.
    #[tracing::instrument(name = "consensus::run_inbound_stream", skip_all)]
    pub(crate) async fn run_inbound_stream(
        &self,
        ctx: &ctx::Ctx,
        mut stream: noise::Stream,
    ) -> anyhow::Result<()> {
        let peer =
            handshake::inbound(ctx, &self.key, self.gossip.genesis_hash(), &mut stream).await?;
        self.inbound.insert(peer.clone(), stream.stats()).await?;
        tracing::trace!("peer = {peer:?}");
        let res = scope::run!(ctx, |ctx, s| async {
            let mut service = rpc::Service::new()
                .add_server(ctx, rpc::ping::Server, rpc::ping::RATE)
                .add_server(ctx, self, self.gossip.cfg.rpc.consensus_rate);
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

    #[tracing::instrument(name = "consensus::run_outbound_stream", skip_all, fields(?peer, %addr))]
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
            self.gossip.genesis_hash(),
            &mut stream,
            peer,
        )
        .await?;
        self.outbound.insert(peer.clone(), stream.stats()).await?;
        tracing::trace!("peer = {peer:?}");
        let consensus_cli =
            rpc::Client::<rpc::consensus::Rpc>::new(ctx, self.gossip.cfg.rpc.consensus_rate);
        let res = scope::run!(ctx, |ctx, s| async {
            let mut service = rpc::Service::new()
                .add_server(ctx, rpc::ping::Server, rpc::ping::RATE)
                .add_client(&consensus_cli);
            if let Some(ping_timeout) = &self.gossip.cfg.ping_timeout {
                let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx, rpc::ping::RATE);
                service = service.add_client(&ping_client);
                s.spawn(async {
                    let ping_client = ping_client;
                    ping_client.ping_loop(ctx, *ping_timeout).await
                });
            }
            s.spawn::<()>(async {
                let mut sub = self.msg_pool.subscribe();
                loop {
                    let call = consensus_cli.reserve(ctx).await?;
                    let msg = sub.recv(ctx).await?.message.clone();

                    s.spawn(async {
                        let req = rpc::consensus::Req(msg);
                        let res = call.call(ctx, &req, RESP_MAX_SIZE).await;
                        if let Err(err) = res {
                            tracing::debug!("{err:#}");
                        }
                        Ok(())
                    });
                }
            });
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

    #[tracing::instrument(skip_all)]
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
                    tracing::debug!("run_loopback_stream(): {err:#}");
                }
            }
            return;
        }
        let addrs = &mut self.gossip.validator_addrs.subscribe();
        let mut addr = None;

        while ctx.is_active() {
            async {
                // Wait for a new address, or retry with the old one after timeout.
                if let Ok(new) =
                    sync::wait_for(&ctx.with_timeout(config::CONNECT_RETRY), addrs, |addrs| {
                        addrs.get(peer).map(|x| x.msg.addr) != addr
                    })
                    .instrument(tracing::trace_span!("wait_for_address"))
                    .await
                {
                    addr = new.get(peer).map(|x| x.msg.addr);
                }
                let Some(addr) = addr else { return };
                if let Err(err) = self.run_outbound_stream(ctx, peer, addr).await {
                    tracing::debug!("run_outbound_stream({peer:?},{addr}): {err:#}");
                }
            }
            .instrument(tracing::trace_span!("maintain_connection_iter"))
            .await;
        }
    }
}
