//! Consensus network is a full graph of connections between all validators.
//! BFT consensus messages are exchanged over this network.
use crate::{config, gossip, io, noise, pool::PoolWatch, preface, rpc};
use anyhow::Context as _;
use rand::seq::SliceRandom;
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};
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

type MsgPoolInner = BTreeMap<usize, Arc<io::ConsensusInputMessage>>;

/// Pool of messages to send.
/// It stores the newest message (with the highest view) of each type.
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
        self.0.send_if_modified(|msgs| {
            // Select a unique ID for the new message: using `last ID+1` is ok (will NOT cause
            // an ID to be reused), because whenever we remove a message, we also insert a message.
            let next = msgs.last_key_value().map_or(0, |(k, _)| k + 1);
            // We first need to check if `msg` should actually be inserted.
            let mut should_insert = true;
            // Remove (at most) 1 message of the same type which is older.
            msgs.retain(|_, v| {
                use validator::ConsensusMsg as M;
                // Messages of other types stay in the pool.
                // TODO(gprusak): internals of `ConsensusMsg` are essentially
                // an implementation detail of the bft crate. Consider moving
                // this logic there.
                match (&v.message.msg, &msg.message.msg) {
                    (M::ReplicaPrepare(_), M::ReplicaPrepare(_)) => {}
                    (M::ReplicaCommit(_), M::ReplicaCommit(_)) => {}
                    (M::LeaderPrepare(_), M::LeaderPrepare(_)) => {}
                    (M::LeaderCommit(_), M::LeaderCommit(_)) => {}
                    _ => return true,
                }
                // If pool contains a message of the same type which is newer,
                // then our message shouldn't be inserted.
                if v.message.msg.view().number >= msg.message.msg.view().number {
                    should_insert = false;
                    return true;
                }
                // An older message of the same type should be removed.
                false
            });
            if should_insert {
                msgs.insert(next, msg);
            }
            // Notify receivers iff we have actually inserted a message.
            should_insert
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
    pub(crate) inbound: PoolWatch<validator::PublicKey, ()>,
    /// Set of the currently open outbound connections.
    pub(crate) outbound: PoolWatch<validator::PublicKey, ()>,
    /// Messages to be sent to validators.
    pub(crate) msg_pool: MsgPool,
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
        // TODO(gprusak): disconnection means that there message was rejected OR
        // that bft actor is missing (in tests), which leads to unnecessary disconnects.
        let _ = recv.recv_or_disconnected(ctx).await?;
        Ok(rpc::consensus::Resp)
    }
}

impl Network {
    /// Constructs a new consensus network state.
    pub(crate) fn new(gossip: Arc<gossip::Network>) -> Option<Arc<Self>> {
        let key = gossip.cfg.validator_key.clone()?;
        let validators: HashSet<_> = gossip.genesis().validators.iter().cloned().collect();
        Some(Arc::new(Self {
            key,
            inbound: PoolWatch::new(validators.clone(), 0),
            outbound: PoolWatch::new(validators.clone(), 0),
            gossip,
            msg_pool: MsgPool::new(),
        }))
    }

    /// Performs handshake of an inbound stream.
    /// Closes the stream if there is another inbound stream opened from the same validator.
    #[tracing::instrument(level = "info", name = "consensus", skip_all)]
    pub(crate) async fn run_inbound_stream(
        &self,
        ctx: &ctx::Ctx,
        mut stream: noise::Stream,
    ) -> anyhow::Result<()> {
        let peer =
            handshake::inbound(ctx, &self.key, self.gossip.genesis().hash(), &mut stream).await?;
        self.inbound.insert(peer.clone(), ()).await?;
        tracing::info!("peer = {peer:?}");
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

    #[tracing::instrument(level = "info", name = "consensus", skip_all)]
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
        self.outbound.insert(peer.clone(), ()).await?;
        tracing::info!("peer = {peer:?}");
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
                    let msg = loop {
                        let msg = sub.recv(ctx).await?;
                        match &msg.recipient {
                            io::Target::Broadcast => {}
                            io::Target::Validator(recipient) if recipient == peer => {}
                            _ => continue,
                        }
                        break msg.message.clone();
                    };
                    s.spawn(async {
                        let req = rpc::consensus::Req(msg);
                        let res = call.call(ctx, &req, RESP_MAX_SIZE).await;
                        if let Err(err) = res {
                            tracing::info!("{err:#}");
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
            .instrument(tracing::info_span!("loopback", ?addr))
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
                .instrument(tracing::info_span!("out", ?addr))
                .await
            {
                tracing::info!("run_outbound_stream({peer:?},{addr}): {err:#}");
            }
        }
    }
}
