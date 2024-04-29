#![allow(dead_code)]
use super::*;
use crate::{frame, mux, noise, preface, rpc, Config, GossipConfig};
use anyhow::Context as _;
use rand::Rng as _;
use std::collections::BTreeMap;
use zksync_concurrency::{ctx, limiter};
use zksync_consensus_roles::validator;

/// Connection.
pub(super) struct Conn {
    accept: BTreeMap<mux::CapabilityId, Arc<mux::StreamQueue>>,
    connect: BTreeMap<mux::CapabilityId, Arc<mux::StreamQueue>>,
}

/// Background task of the connection.
pub(super) struct ConnRunner {
    mux: mux::Mux,
    stream: noise::Stream,
}

impl ConnRunner {
    /// Runs the background task of the connection.
    pub(super) async fn run(self, ctx: &ctx::Ctx) -> Result<(), mux::RunError> {
        self.mux.run(ctx, self.stream).await
    }
}

fn mux_entry<R: rpc::Rpc>(ctx: &ctx::Ctx) -> (mux::CapabilityId, Arc<mux::StreamQueue>) {
    (
        R::CAPABILITY_ID,
        mux::StreamQueue::new(ctx, R::INFLIGHT, limiter::Rate::INF),
    )
}

/// Establishes an anonymous gossipnet connection to a peer.
pub(super) async fn connect(
    ctx: &ctx::Ctx,
    peer: &Config,
    genesis: validator::GenesisHash,
) -> ctx::Result<(Conn, ConnRunner)> {
    assert!(peer.gossip.dynamic_inbound_limit > 0);
    let addr = peer
        .public_addr
        .resolve(ctx)
        .await?
        .context("peer.public_addr.resolve()")?[0];
    let mut stream = preface::connect(ctx, addr, preface::Endpoint::GossipNet)
        .await
        .context("preface::connect()")?;
    let cfg = GossipConfig {
        key: ctx.rng().gen(),
        dynamic_inbound_limit: 0,
        static_outbound: [].into(),
        static_inbound: [].into(),
    };
    handshake::outbound(ctx, &cfg, genesis, &mut stream, &peer.gossip.key.public())
        .await
        .context("handshake::outbound()")?;
    let conn = Conn {
        accept: [
            mux_entry::<rpc::get_block::Rpc>(ctx),
            mux_entry::<rpc::push_block_store_state::Rpc>(ctx),
        ]
        .into(),
        connect: [
            mux_entry::<rpc::get_block::Rpc>(ctx),
            mux_entry::<rpc::push_block_store_state::Rpc>(ctx),
        ]
        .into(),
    };
    let mux = mux::Mux {
        cfg: Arc::new(rpc::MUX_CONFIG.clone()),
        accept: conn.accept.clone(),
        connect: conn.connect.clone(),
    };
    Ok((conn, ConnRunner { mux, stream }))
}

impl Conn {
    /// Opens a server-side stream.
    pub(super) async fn open_server<R: rpc::Rpc>(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::OrCanceled<ServerStream<R>> {
        Ok(ServerStream(
            self.connect
                .get(&R::CAPABILITY_ID)
                .unwrap()
                .open(ctx)
                .await?,
            std::marker::PhantomData,
        ))
    }

    /// Opens a client-side stream.
    pub(super) async fn open_client<R: rpc::Rpc>(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::OrCanceled<ClientStream<R>> {
        Ok(ClientStream(
            self.accept
                .get(&R::CAPABILITY_ID)
                .unwrap()
                .open(ctx)
                .await?,
            std::marker::PhantomData,
        ))
    }
}

/// Client side stream.
pub(super) struct ClientStream<R>(mux::Stream, std::marker::PhantomData<R>);
/// Server side stream.
pub(super) struct ServerStream<R>(mux::Stream, std::marker::PhantomData<R>);

impl<R: rpc::Rpc> ClientStream<R> {
    /// Sends a request.
    pub(super) async fn send(&mut self, ctx: &ctx::Ctx, req: &R::Req) -> anyhow::Result<()> {
        frame::mux_send_proto(ctx, &mut self.0.write, req).await?;
        self.0.write.flush(ctx).await?;
        Ok(())
    }

    /// Receives a response.
    pub(super) async fn recv(mut self, ctx: &ctx::Ctx) -> anyhow::Result<R::Resp> {
        Ok(frame::mux_recv_proto(ctx, &mut self.0.read, usize::MAX)
            .await?
            .0)
    }
}

impl<R: rpc::Rpc> ServerStream<R> {
    /// Sends a response.
    pub(super) async fn send(mut self, ctx: &ctx::Ctx, resp: &R::Resp) -> anyhow::Result<()> {
        frame::mux_send_proto(ctx, &mut self.0.write, resp).await?;
        self.0.write.flush(ctx).await?;
        Ok(())
    }

    /// Receives a request.
    pub(super) async fn recv(&mut self, ctx: &ctx::Ctx) -> anyhow::Result<R::Req> {
        Ok(frame::mux_recv_proto(ctx, &mut self.0.read, usize::MAX)
            .await?
            .0)
    }
}
