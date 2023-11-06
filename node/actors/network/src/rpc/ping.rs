//! Defines an RPC for sending ping messages.
use crate::{mux, rpc::Rpc as _};
use anyhow::Context as _;
use rand::Rng;
use zksync_concurrency::{ctx, limiter, time};
use zksync_consensus_schema as schema;
use zksync_consensus_schema::{proto::network::ping as proto, required, ProtoFmt};

/// Ping RPC.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY_ID: mux::CapabilityId = 2;
    const INFLIGHT: u32 = 1;
    const RATE: limiter::Rate = limiter::Rate {
        burst: 1,
        refresh: time::Duration::seconds(10),
    };
    const METHOD: &'static str = "ping";
    type Req = Req;
    type Resp = Resp;
}

/// Canonical Ping server implementation,
/// which responds with data from the request.
pub(crate) struct Server;

#[async_trait::async_trait]
impl super::Handler<Rpc> for Server {
    async fn handle(&self, _ctx: &ctx::Ctx, req: Req) -> anyhow::Result<Resp> {
        Ok(Resp(req.0))
    }
}

impl super::Client<Rpc> {
    /// Calls ping RPC every `Rpc.RATE.refresh`.
    /// Returns an error if any single ping request fails or
    /// exceeds `timeout`.
    pub(crate) async fn ping_loop(
        &self,
        ctx: &ctx::Ctx,
        timeout: time::Duration,
    ) -> anyhow::Result<()> {
        loop {
            ctx.sleep(Rpc::RATE.refresh).await?;
            let ctx = &ctx.with_timeout(timeout);
            let req = Req(ctx.rng().gen());
            let resp = self.call(ctx, &req).await.context("ping")?;
            if req.0 != resp.0 {
                anyhow::bail!("bad ping response");
            }
        }
    }
}

/// Ping request, contains random data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req(pub(crate) [u8; 32]);

/// Ping response, should contain the same data
/// as the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resp(pub(crate) [u8; 32]);

impl ProtoFmt for Req {
    type Proto = proto::PingReq;
    fn max_size() -> usize {
        schema::kB
    }
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(required(&r.data)?[..].try_into()?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            data: Some(self.0.into()),
        }
    }
}

impl ProtoFmt for Resp {
    type Proto = proto::PingResp;
    fn max_size() -> usize {
        schema::kB
    }
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(required(&r.data)?[..].try_into()?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            data: Some(self.0.into()),
        }
    }
}
