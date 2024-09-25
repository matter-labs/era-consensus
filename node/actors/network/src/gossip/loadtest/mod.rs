//! Loadtest of the gossip endpoint of a node.
use crate::{gossip, mux, noise, preface, rpc, testonly};
use anyhow::Context as _;
use async_trait::async_trait;
use rand::Rng;
use zksync_concurrency::{ctx, error::Wrap as _, limiter, net, scope, sync, time};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{BlockStoreState};
use zksync_protobuf::kB;

#[cfg(test)]
mod tests;

struct PushBlockStoreStateServer(sync::watch::Sender<Option<BlockStoreState>>);

impl<'a> PushBlockStoreStateServer {
    fn new() -> Self {
        Self(sync::watch::channel(None).0)
    }

    /// Waits until the peer tells us which blocks it has.
    /// Guaranteed to return a nonempty range of `BlockNumber`s.
    async fn wait_for_peer(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::OrCanceled<std::ops::Range<validator::BlockNumber>> {
        let sub = &mut self.0.subscribe();
        // unwraps are safe, because we have checked that in `wait_for`.
        let state =
            sync::wait_for(ctx, sub, |s| (|| s.as_ref()?.last.as_ref())().is_some()).await?;
        let state = state.as_ref().unwrap();
        Ok(state.first..state.last.as_ref().unwrap().number() + 1)
    }
}

#[async_trait]
impl rpc::Handler<rpc::push_block_store_state::Rpc> for &PushBlockStoreStateServer {
    fn max_req_size(&self) -> usize {
        10 * kB
    }
    async fn handle(
        &self,
        _ctx: &ctx::Ctx,
        req: rpc::push_block_store_state::Req,
    ) -> anyhow::Result<()> {
        self.0.send_replace(Some(req.0));
        Ok(())
    }
}

/// Traffic pattern to generate.
pub enum TrafficPattern {
    /// Fetch always a random available blocks
    /// Bypasess all the caches that a node can have.
    /// This is an adversary traffic pattern.
    Random,
    /// Fetch blocks sequentially starting from a random one.
    /// Bypasses era-consensus cache, but is potentially DB friendly.
    /// This is the expected traffic pattern for the nodes that are syncing up.
    Sequential,
    /// Fetch always the latest available block.
    /// Hits the era-consensus cache - it is an in-memory query.
    /// This is the expected traffic pattern for the nodes that are up to date.
    Latest,
}

/// Loadtest saturating the unauthenticated gossip connections of a peer
/// and spamming it with maximal get_blocks throughput.
pub struct Loadtest {
    /// Address of the peer to spam.
    pub addr: net::Host,
    /// Key of the peer to spam.
    pub peer: node::PublicKey,
    /// Genesis of the chain.
    pub genesis: validator::Genesis,
    /// Traffic pattern to generate.
    pub traffic_pattern: TrafficPattern,
    /// Channel to send the received responses to.
    pub output: Option<ctx::channel::Sender<Option<validator::Block>>>,
}

impl Loadtest {
    async fn connect(&self, ctx: &ctx::Ctx) -> ctx::Result<noise::Stream> {
        let addr = *self
            .addr
            .resolve(ctx)
            .await?
            .context("resolve()")?
            .first()
            .context("resolution failed")?;
        let mut stream = preface::connect(ctx, addr, preface::Endpoint::GossipNet)
            .await
            .context("connect()")?;
        let cfg = testonly::make_config(ctx.rng().gen());
        gossip::handshake::outbound(ctx, &cfg, self.genesis.hash(), &mut stream, &self.peer)
            .await
            .context("handshake")?;
        Ok(stream)
    }

    async fn spam(&self, ctx: &ctx::Ctx, stream: noise::Stream) -> ctx::Result<()> {
        let push_block_store_state_server = PushBlockStoreStateServer::new();
        let get_block_client = rpc::Client::<rpc::get_block::Rpc>::new(ctx, limiter::Rate::INF);
        let mut rng = ctx.rng();
        scope::run!(ctx, |ctx, s| async {
            let service = rpc::Service::new()
                .add_client(&get_block_client)
                // We listen to PushBlockStore messages to know which blocks we can fetch.
                .add_server(ctx, &push_block_store_state_server, limiter::Rate::INF)
                // We respond to pings, so that peer considers connection healthy.
                .add_server(ctx, rpc::ping::Server, limiter::Rate::INF);
            s.spawn(async {
                match service.run(ctx, stream).await {
                    // Peer didn't want to talk with us.
                    // This allows to avoid "EarlyEof" errors from being logged all the time.
                    Ok(()) | Err(mux::RunError::Protocol(_)) => Ok(()),
                    Err(err) => Err(err.into()),
                }
            });
            let mut next = validator::BlockNumber(0);
            loop {
                let call = get_block_client.reserve(ctx).await?;
                let range = push_block_store_state_server.wait_for_peer(ctx).await?;
                let mut sample =
                    || validator::BlockNumber(rng.gen_range(range.start.0..range.end.0));
                match self.traffic_pattern {
                    // unwrap is safe, because the range is guaranteed to be non-empty by
                    // `wait_for_peer`.
                    TrafficPattern::Latest => next = range.end.prev().unwrap(),
                    TrafficPattern::Random => next = sample(),
                    TrafficPattern::Sequential => {
                        next = next + 1;
                        if !range.contains(&next) {
                            next = sample();
                        }
                    }
                }
                let req = rpc::get_block::Req(next);
                s.spawn(async {
                    let req = req;
                    let resp = call.call(ctx, &req, usize::MAX).await.wrap("call")?;
                    if let Some(send) = &self.output {
                        send.send(ctx, resp.0).await?;
                    }
                    Ok(())
                });
            }
        })
        .await
    }

    /// Run the loadtest.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let res = scope::run!(ctx, |ctx, s| async {
            loop {
                match self.connect(ctx).await {
                    Ok(stream) => {
                        s.spawn(async {
                            if let Err(err) = self.spam(ctx, stream).await {
                                tracing::warn!("spam(): {err:#}");
                            }
                            Ok(())
                        });
                    }
                    Err(err) => {
                        tracing::warn!("connect(): {err:#}");
                        ctx.sleep(time::Duration::seconds(10)).await?;
                    }
                }
            }
        })
        .await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}
