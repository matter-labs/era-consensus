//! Defines an RPC for sending ping messages.
use crate::{proto::ping as proto};
use anyhow::Context as _;
use rand::Rng;
use zksync_concurrency::{ctx, limiter, time};
use zksync_protobuf::{kB, required, ProtoFmt};
use super::Capability;

/// Ping RPC.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY : Capability = Capability::Ping; 
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "ping";
    type Req = Req;
    type Resp = Resp;
}

/// Hardcoded expected rate supported by the server.
/// This needs to be part of the protocol, so that both parties agree on when
/// connection is alive.
pub(crate) const RATE: limiter::Rate = limiter::Rate {
    burst: 2,
    refresh: time::Duration::seconds(1),
};

/// Canonical Ping server implementation,
/// which responds with data from the request.
pub(crate) struct Server;

#[async_trait::async_trait]
impl super::Handler<Rpc> for Server {
    fn max_req_size(&self) -> usize {
        kB
    }
    async fn handle(&self, _ctx: &ctx::Ctx, req: Req) -> anyhow::Result<Resp> {
        Ok(Resp(req.0))
    }
}

impl super::Client<Rpc> {
    /// Sends a ping every `timeout`.
    /// Returns an error if any single ping request fails or
    /// exceeds `timeout`.
    pub(crate) async fn ping_loop(
        &self,
        ctx: &ctx::Ctx,
        timeout: time::Duration,
    ) -> anyhow::Result<()> {
        loop {
            let req = Req(ctx.rng().gen());
            let resp = self
                .call(&ctx.with_timeout(timeout), &req, kB)
                .await
                .context("ping")?;
            if req.0 != resp.0 {
                anyhow::bail!("bad ping response");
            }
            if let Err(ctx::Canceled) = ctx.sleep(timeout).await {
                return Ok(());
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
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(required(&r.data)?[..].try_into()?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            data: Some(self.0.into()),
        }
    }
}
